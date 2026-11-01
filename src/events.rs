//! Messages from worker threads back to the UI thread.
//!
//! Workers never touch UI state directly: they send `AppEvent`s over a
//! channel and wake the UI with `request_repaint`, so the window stays
//! responsive no matter how long an operation takes.

use std::sync::mpsc::Sender;

use eframe::egui;

use crate::sys::util::local_time_hms;

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

/// Current on/off state of each toggle, read from the system.
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
    JobFinished(Job, Result<(), String>),
    State(TweakState),
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
        self.send(AppEvent::Log(LogEntry { time: local_time_hms(), level, message: message.into() }));
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

    /// Runs `work` on a new thread and reports its result as `JobFinished`.
    /// A panic inside the job is caught and reported instead of killing the app.
    pub fn spawn_job(&self, job: Job, work: impl FnOnce(&Logger) -> Result<(), String> + Send + 'static) {
        let logger = self.clone();
        let spawned = std::thread::Builder::new().name(format!("{job:?}")).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&logger)))
                .unwrap_or_else(|_| Err("internal error (the operation panicked)".into()));
            if let Err(e) = &result {
                logger.error(e.clone());
            }
            logger.send(AppEvent::JobFinished(job, result));
        });
        if let Err(e) = spawned {
            self.error(format!("Could not start background thread: {e}"));
            self.send(AppEvent::JobFinished(job, Err(e.to_string())));
        }
    }
}
