//! Messages from worker threads back to the UI thread, and the job runner.
//!
//! Workers never touch UI state directly: they send `AppEvent`s over a
//! channel and wake the UI with `request_repaint`, so the window stays
//! responsive no matter how long an operation takes.

use std::collections::HashSet;
use std::sync::mpsc::Sender;

use eframe::egui;

use crate::sys::format::local_time_hms;
use crate::sys::tray::TrayCommand;

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
    Copilot,
    Telemetry,
    Updates,
    LowPower,
    DetectState,
    ProcessAction,
}

/// Current on/off state of each switch, read from the system.
#[derive(Clone, Debug, Default)]
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
        self.ctx.request_repaint();
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
pub struct Jobs {
    pub log: Logger,
    busy: HashSet<Job>,
}

impl Jobs {
    pub fn new(log: Logger) -> Self {
        Self {
            log,
            busy: HashSet::new(),
        }
    }

    pub fn is_busy(&self, job: Job) -> bool {
        self.busy.contains(&job)
    }

    /// True while any user-visible job runs (state detection doesn't count).
    pub fn any_visible_busy(&self) -> bool {
        self.busy.iter().any(|j| *j != Job::DetectState)
    }

    /// Called by the UI when a `JobFinished` event arrives.
    pub fn finished(&mut self, job: Job) {
        self.busy.remove(&job);
    }

    /// Runs `work` on a new thread and reports its result as `JobFinished`.
    /// Does nothing if the same job is already running. A panic inside the
    /// job is caught and reported instead of taking the app down.
    pub fn start(
        &mut self,
        job: Job,
        work: impl FnOnce(&Logger) -> Result<(), String> + Send + 'static,
    ) {
        if !self.busy.insert(job) {
            return;
        }
        let logger = self.log.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("{job:?}"))
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&logger)))
                        .unwrap_or_else(|_| Err("internal error (the operation panicked)".into()));
                if let Err(e) = &result {
                    logger.error(e.clone());
                }
                logger.send(AppEvent::JobFinished(job));
            });
        if let Err(e) = spawned {
            self.busy.remove(&job);
            self.log
                .error(format!("Could not start a background thread: {e}"));
        }
    }
}
