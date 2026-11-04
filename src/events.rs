//! Messages from worker threads back to the UI thread, and the job runner.
//!
//! Workers never touch UI state directly: they send `AppEvent`s over a
//! channel and wake the UI with `request_repaint`, so the window stays
//! responsive no matter how long an operation takes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, ViewportId};

use crate::i18n::tr;
use crate::sys::format::local_time_hms;
use crate::sys::tray::TrayCommand;
use crate::sys::updater::Release;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: String,
    pub level: Level,
    pub message: String,
}

/// Background jobs the UI tracks (one of each can run at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Job {
    CleanRam,
    #[cfg_attr(not(windows), allow(dead_code))]
    Copilot,
    #[cfg_attr(not(windows), allow(dead_code))]
    Telemetry,
    #[cfg_attr(not(windows), allow(dead_code))]
    Updates,
    LowPower,
    DetectState,
    ProcessAction,
    Startup,
    UpdateCheck,
    UpdateInstall,
    Changelog,
    #[cfg(any(debug_assertions, feature = "stress-test"))]
    StressTest,
}

/// Current on/off state of each switch, read from the system.
#[derive(Clone, Debug, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct TweakState {
    pub copilot_disabled: bool,
    pub telemetry_disabled: bool,
    pub updates_blocked: bool,
    pub paused_until: Option<String>,
    pub low_power: bool,
}

pub enum AppEvent {
    Log(LogEntry),
    /// A job ended. Its errors were already logged by the job runner.
    JobFinished(Job),
    State(TweakState),
    Tray(TrayCommand),
    /// A newer release exists. `manual` is true when the user pressed
    /// "Check for updates", so "up to date" is worth saying too.
    UpdateAvailable(Release),
    UpToDate {
        manual: bool,
    },
    /// Quit and start this exe (the new version after an update, or the
    /// same one to apply a setting that needs a restart).
    Restart(PathBuf),
}

/// Cloneable handle workers use to report progress.
#[derive(Clone)]
pub struct Logger {
    tx: Sender<AppEvent>,
    ctx: egui::Context,
}

impl Logger {
    pub fn new(tx: Sender<AppEvent>, ctx: egui::Context) -> Self {
        Self { tx, ctx }
    }

    pub fn send(&self, event: AppEvent) {
        // The receiver only disappears when the app is closing; nothing to do then.
        let _ = self.tx.send(event);
        // Events are handled in the main window's pass, which also runs while
        // that window is hidden in the tray.
        self.ctx.request_repaint_of(ViewportId::ROOT);
    }

    fn log(&self, level: Level, message: impl Into<String>) {
        self.send(AppEvent::Log(LogEntry {
            time: local_time_hms(),
            level,
            message: message.into(),
        }));
    }

    pub fn info(&self, m: impl Into<String>) {
        self.log(Level::Info, m)
    }
    pub fn success(&self, m: impl Into<String>) {
        self.log(Level::Success, m)
    }
    pub fn warn(&self, m: impl Into<String>) {
        self.log(Level::Warn, m)
    }
    pub fn error(&self, m: impl Into<String>) {
        self.log(Level::Error, m)
    }
}

/// Starts background jobs and remembers which ones are still running.
/// Cheap to clone, so detached windows can start jobs of their own.
#[derive(Clone)]
pub struct Jobs {
    pub log: Logger,
    busy: Arc<Mutex<HashSet<Job>>>,
}

impl Jobs {
    pub fn new(log: Logger) -> Self {
        Self {
            log,
            busy: Arc::default(),
        }
    }

    fn busy(&self) -> std::sync::MutexGuard<'_, HashSet<Job>> {
        // A panic while holding this lock can't leave the set half-updated.
        self.busy.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_busy(&self, job: Job) -> bool {
        self.busy().contains(&job)
    }

    /// True while any user-visible job runs (background checks don't count).
    pub fn any_visible_busy(&self) -> bool {
        self.busy()
            .iter()
            .any(|j| {
                !matches!(
                    j,
                    Job::DetectState | Job::UpdateCheck | Job::Startup | Job::Changelog
                )
            })
    }

    /// Runs `work` on a new thread and reports its result as `JobFinished`.
    /// Does nothing if the same job is already running. A panic inside the
    /// job is caught and reported instead of taking the app down.
    pub fn start(
        &self,
        job: Job,
        work: impl FnOnce(&Logger) -> Result<(), String> + Send + 'static,
    ) {
        if !self.busy().insert(job) {
            return;
        }
        let logger = self.log.clone();
        let busy = self.busy.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("{job:?}"))
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&logger)))
                        .unwrap_or_else(|_| {
                            Err(tr("internal error (the operation panicked)").into())
                        });
                if let Err(e) = &result {
                    logger.error(e.clone());
                }
                busy.lock().unwrap_or_else(|e| e.into_inner()).remove(&job);
                logger.send(AppEvent::JobFinished(job));
            });
        if let Err(e) = spawned {
            self.busy().remove(&job);
            self.log.error(crate::i18n::trf(
                "Could not start a background thread: {0}",
                &[&e],
            ));
        }
    }
}
