//! The RustCleanup window: layout, state and dispatch of background jobs.
//!
//! The UI thread only ever draws and reads channels. Every system call that
//! could take more than a few milliseconds runs on a worker thread (see
//! `Logger::spawn_job`), so the window never freezes.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::events::{AppEvent, Job, Level, LogEntry, Logger, TweakState};
use crate::sys::disk::{self, Drive, ScanEvent, ScanMode};
use crate::sys::processes::{self, ProcInfo, ProcKind, ProcessMonitor};
use crate::sys::ram::{self, MemStatus};
use crate::sys::tools::TOOLS;
use crate::sys::util::{fmt_bytes, is_elevated};
use crate::sys::{power, tweaks};
use crate::widgets::{self, ACCENT, ERR_RED, OK_GREEN, WARN_AMBER};

/// Maximum lines kept in the activity log and the chkdsk terminal.
const MAX_LOG: usize = 500;
const MAX_SCAN_LINES: usize = 10_000;

/// A running or finished chkdsk session shown in its own window.
struct ScanSession {
    title: String,
    lines: Vec<String>,
    /// Current progress line (chkdsk redraws it in place with `\r`).
    progress: Option<String>,
    rx: Receiver<ScanEvent>,
    pid: u32,
    started: Instant,
    finished: Option<(Result<i32, String>, Duration)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Name,
    Cpu,
    Memory,
}

/// State of the Optimize Task window.
struct TaskWindow {
    monitor: ProcessMonitor,
    filter: String,
    show_background: bool,
    selected: HashSet<u32>,
    sort: SortBy,
}

/// Process action chosen in the Optimize Task window.
#[derive(Clone, Copy, Debug)]
enum ProcAction {
    Optimize,
    Suspend,
    Resume,
    End,
}

impl ProcAction {
    fn verb(self) -> &'static str {
        match self {
            ProcAction::Optimize => "Optimized",
            ProcAction::Suspend => "Suspended",
            ProcAction::Resume => "Resumed",
            ProcAction::End => "Ended",
        }
    }
}

pub struct RustCleanupApp {
    rx: Receiver<AppEvent>,
    logger: Logger,
    log: Vec<LogEntry>,
    busy: HashSet<Job>,
    elevated: bool,

    state: TweakState,
    state_loaded: bool,

    mem: Option<MemStatus>,
    mem_checked: Instant,
    purge_standby: bool,

    restart_explorer: bool,
    pause_updates: bool,

    drives: Vec<Drive>,
    drive_idx: usize,
    scan_mode: ScanMode,
    scan: Option<ScanSession>,

    tasks: Option<TaskWindow>,
    /// PIDs we suspended, so they can be shown and resumed (also on exit).
    suspended: HashSet<u32>,
    /// Processes waiting for the user to confirm "End task".
    confirm_end: Option<Vec<(u32, String)>>,

    tool_idx: usize,
}

impl RustCleanupApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let (tx, rx): (Sender<AppEvent>, Receiver<AppEvent>) = mpsc::channel();
        let logger = Logger::new(tx, cc.egui_ctx.clone());
        let elevated = is_elevated();

        let mut app = Self {
            rx,
            logger,
            log: Vec::new(),
            busy: HashSet::new(),
            elevated,
            state: TweakState::default(),
            state_loaded: false,
            mem: ram::mem_status(),
            mem_checked: Instant::now(),
            purge_standby: true,
            restart_explorer: true,
            pause_updates: true,
            drives: disk::list_drives(),
            drive_idx: 0,
            scan_mode: ScanMode::ReadOnly,
            scan: None,
            tasks: None,
            suspended: HashSet::new(),
            confirm_end: None,
            tool_idx: 0,
        };
        // Pre-select the system drive.
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        app.drive_idx = app.drives.iter().position(|d| d.volume().eq_ignore_ascii_case(&system)).unwrap_or(0);

        if elevated {
            app.logger.info("RustCleanup started with Administrator rights.");
        } else {
            app.logger.warn("Not running as Administrator: most tweaks will fail. Restart RustCleanup and accept the UAC prompt.");
        }
        app.detect_state();
        app
    }

    /// Re-reads every toggle's real state from the system, in the background.
    fn detect_state(&mut self) {
        self.start_job(Job::DetectState, |log| {
            let state = TweakState {
                copilot_disabled: tweaks::copilot_disabled(),
                telemetry_disabled: tweaks::telemetry_disabled(),
                updates_blocked: tweaks::forced_updates_blocked(),
                paused_until: tweaks::paused_until(),
                low_power: power::low_power_active(),
            };
            log.send(AppEvent::State(state));
            Ok(())
        });
    }

    fn start_job(&mut self, job: Job, work: impl FnOnce(&Logger) -> Result<(), String> + Send + 'static) {
        if self.busy.insert(job) {
            self.logger.spawn_job(job, work);
        }
    }

    fn is_busy(&self, job: Job) -> bool {
        self.busy.contains(&job)
    }

    /// Drains worker messages. Called once per frame before drawing.
    fn pump_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                AppEvent::Log(entry) => {
                    self.log.push(entry);
                    if self.log.len() > MAX_LOG {
                        self.log.drain(..self.log.len() - MAX_LOG);
                    }
                }
                AppEvent::State(state) => {
                    self.state = state;
                    self.state_loaded = true;
                }
                AppEvent::JobFinished(job, _result) => {
                    self.busy.remove(&job);
                    match job {
                        // Tweak toggles always resync with the real system state,
                        // so a failed apply never leaves the switch lying.
                        Job::Copilot | Job::Telemetry | Job::Updates | Job::LowPower => self.detect_state(),
                        Job::CleanRam => self.mem = ram::mem_status(),
                        _ => {}
                    }
                }
            }
        }

        if let Some(scan) = &mut self.scan {
            while let Ok(event) = scan.rx.try_recv() {
                match event {
                    ScanEvent::Line(line) => {
                        // A finished progress line is replaced by the real line.
                        scan.progress = None;
                        scan.lines.push(line);
                        if scan.lines.len() > MAX_SCAN_LINES {
                            scan.lines.drain(..scan.lines.len() - MAX_SCAN_LINES);
                        }
                    }
                    ScanEvent::Progress(p) if !p.trim().is_empty() => scan.progress = Some(p),
                    ScanEvent::Progress(_) => {}
                    ScanEvent::Finished(result) => {
                        let elapsed = scan.started.elapsed();
                        match &result {
                            Ok(code) => self.logger.info(format!(
                                "{} finished with exit code {code}: {}",
                                scan.title,
                                disk::describe_exit(*code)
                            )),
                            Err(e) => self.logger.error(format!("{} failed: {e}", scan.title)),
                        }
                        scan.finished = Some((result, elapsed));
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Header and log
    // ------------------------------------------------------------------

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("⚡").size(26.0).color(ACCENT));
            ui.vertical(|ui| {
                ui.label(RichText::new("RustCleanup").size(22.0).strong());
                ui.label(RichText::new("Windows 11 optimizer & debloater").small().weak());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color) = if self.elevated {
                    ("● Administrator", OK_GREEN)
                } else {
                    ("● Not elevated", ERR_RED)
                };
                ui.label(RichText::new(text).color(color).strong());
                if self.busy.iter().any(|j| *j != Job::DetectState) {
                    ui.add(egui::Spinner::new());
                    ui.label(RichText::new("Working…").weak());
                }
            });
        });
        if !self.elevated {
            ui.colored_label(
                ERR_RED,
                "RustCleanup is not elevated. Close it and run RustCleanup.exe again, choosing \"Yes\" on the UAC prompt.",
            );
        }
    }

    fn activity_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Activity").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    self.log.clear();
                }
                if ui.small_button("Copy").clicked() {
                    let text: String =
                        self.log.iter().map(|e| format!("[{}] {}\n", e.time, e.message)).collect();
                    ui.ctx().copy_text(text);
                }
            });
        });
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for entry in &self.log {
                let (tag, color) = match entry.level {
                    Level::Info => ("INFO", ui.visuals().weak_text_color()),
                    Level::Success => (" OK ", OK_GREEN),
                    Level::Warn => ("WARN", WARN_AMBER),
                    Level::Error => ("FAIL", ERR_RED),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&entry.time).monospace().weak());
                    ui.label(RichText::new(tag).monospace().color(color));
                    ui.label(&entry.message);
                });
            }
        });
    }

    // ------------------------------------------------------------------
    // Cards
    // ------------------------------------------------------------------

    fn memory_card(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "Clean RAM", "Trim process working sets and purge cached standby memory", |ui| {
            if let Some(m) = self.mem {
                let frac = m.used() as f32 / m.total.max(1) as f32;
                let color = if m.load_percent >= 85 { ERR_RED } else if m.load_percent >= 65 { WARN_AMBER } else { OK_GREEN };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .fill(color.gamma_multiply(0.85))
                        .text(format!("{} of {} used ({}%)", fmt_bytes(m.used()), fmt_bytes(m.total), m.load_percent)),
                );
            }
            ui.add_space(6.0);
            ui.checkbox(&mut self.purge_standby, "Also purge the standby list (file cache)");
            ui.add_space(6.0);
            let busy = self.is_busy(Job::CleanRam);
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, "Clean RAM", !busy).clicked() {
                    let purge = self.purge_standby;
                    self.logger.info("Cleaning RAM…");
                    self.start_job(Job::CleanRam, move |log| ram::clean_ram(log, purge));
                }
                if busy {
                    ui.add(egui::Spinner::new());
                }
            });
        });
    }

    fn disk_card(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "Scan Disk", "Run chkdsk with live output", |ui| {
            ui.horizontal(|ui| {
                let selected = self.drives.get(self.drive_idx).map(|d| d.display_name()).unwrap_or_else(|| "No drives found".into());
                egui::ComboBox::from_id_salt("drive")
                    .width(ui.available_width() - 40.0)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, d) in self.drives.iter().enumerate() {
                            ui.selectable_value(&mut self.drive_idx, i, d.display_name());
                        }
                    });
                if ui.button("⟳").on_hover_text("Refresh drive list").clicked() {
                    self.drives = disk::list_drives();
                    self.drive_idx = self.drive_idx.min(self.drives.len().saturating_sub(1));
                }
            });
            ui.add_space(4.0);
            egui::ComboBox::from_id_salt("scan_mode")
                .width(ui.available_width())
                .selected_text(self.scan_mode.label())
                .show_ui(ui, |ui| {
                    for mode in ScanMode::ALL {
                        ui.selectable_value(&mut self.scan_mode, mode, mode.label());
                    }
                });
            ui.add_space(6.0);

            let drive = self.drives.get(self.drive_idx).cloned();
            let running = self.scan.as_ref().is_some_and(|s| s.finished.is_none());
            let can_scan = drive.as_ref().is_some_and(|d| d.scannable) && !running;
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, "Run scan", can_scan).clicked()
                    && let Some(d) = drive.clone() {
                        self.start_scan(ui.ctx(), &d);
                    }
                if let Some(d) = &drive
                    && !d.scannable {
                        ui.label(RichText::new("chkdsk can't check this drive type").small().color(WARN_AMBER));
                    }
                if running {
                    ui.add(egui::Spinner::new());
                    ui.label(RichText::new("Scan running").weak());
                }
            });
        });
    }

    fn start_scan(&mut self, ctx: &egui::Context, drive: &Drive) {
        let (tx, rx) = mpsc::channel();
        let repaint_ctx = ctx.clone();
        match disk::start_chkdsk(drive, self.scan_mode, tx, move || repaint_ctx.request_repaint()) {
            Ok(handle) => {
                let title = format!("chkdsk {} ({})", drive.volume(), self.scan_mode.label());
                self.logger.info(format!("Started {title}."));
                self.scan = Some(ScanSession {
                    title,
                    lines: Vec::new(),
                    progress: None,
                    rx,
                    pid: handle.pid,
                    started: Instant::now(),
                    finished: None,
                });
            }
            Err(e) => self.logger.error(e),
        }
    }

    fn tasks_card(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "Optimize Task", "See what is running and tame resource-heavy apps", |ui| {
            ui.label("Trim memory, lower priority, suspend or end selected processes. System-critical processes are protected.");
            ui.add_space(6.0);
            let label = if self.tasks.is_some() { "Window open" } else { "Open task list" };
            if widgets::primary_button(ui, label, self.tasks.is_none()).clicked() {
                let ctx = ui.ctx().clone();
                self.tasks = Some(TaskWindow {
                    monitor: ProcessMonitor::start(move || ctx.request_repaint()),
                    filter: String::new(),
                    show_background: true,
                    selected: HashSet::new(),
                    sort: SortBy::Memory,
                });
            }
        });
    }

    /// One labelled switch row. Returns the requested new value when clicked.
    fn toggle_row(ui: &mut egui::Ui, title: &str, detail: &str, on: bool, busy: bool, ready: bool) -> Option<bool> {
        let mut result = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 70.0);
                ui.label(RichText::new(title).strong());
                ui.label(RichText::new(detail).small().weak());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::toggle_switch(ui, on, !busy && ready).clicked() {
                    result = Some(!on);
                }
                if busy {
                    ui.add(egui::Spinner::new());
                }
            });
        });
        result
    }

    fn privacy_card(&mut self, ui: &mut egui::Ui) {
        let ready = self.state_loaded;
        widgets::card(ui, "Privacy & Updates", "Policies are fully reversible: switch off to restore defaults", |ui| {
            // --- Copilot ---
            let busy = self.is_busy(Job::Copilot);
            if let Some(disable) = Self::toggle_row(
                ui,
                "Disable Copilot",
                "Sets the TurnOffWindowsCopilot policy (HKCU + HKLM) and hides the taskbar button",
                self.state.copilot_disabled,
                busy,
                ready,
            ) {
                let restart = self.restart_explorer;
                self.state.copilot_disabled = disable;
                self.start_job(Job::Copilot, move |log| tweaks::set_copilot_disabled(log, disable, restart));
            }
            ui.checkbox(&mut self.restart_explorer, RichText::new("Restart Explorer to apply instantly").small());
            ui.separator();

            // --- Telemetry ---
            let busy = self.is_busy(Job::Telemetry);
            if let Some(disable) = Self::toggle_row(
                ui,
                "Disable Telemetry",
                "AllowTelemetry=0, stops DiagTrack & dmwappushservice, disables data-collection tasks",
                self.state.telemetry_disabled,
                busy,
                ready,
            ) {
                self.state.telemetry_disabled = disable;
                self.start_job(Job::Telemetry, move |log| tweaks::set_telemetry_disabled(log, disable));
            }
            ui.separator();

            // --- Windows Update ---
            let busy = self.is_busy(Job::Updates);
            if let Some(block) = Self::toggle_row(
                ui,
                "No Forced Updates",
                "Notify-only updates and no automatic reboots while you're signed in",
                self.state.updates_blocked,
                busy,
                ready,
            ) {
                let pause = self.pause_updates;
                self.state.updates_blocked = block;
                self.start_job(Job::Updates, move |log| tweaks::set_forced_updates_blocked(log, block, pause));
            }
            ui.add_enabled(
                !self.state.updates_blocked,
                egui::Checkbox::new(
                    &mut self.pause_updates,
                    RichText::new(format!("Also pause updates for {} days", tweaks::PAUSE_DAYS)).small(),
                ),
            );
            if let Some(until) = &self.state.paused_until {
                ui.label(RichText::new(format!("Updates paused until {until}")).small().color(WARN_AMBER));
            }
        });
    }

    fn power_card(&mut self, ui: &mut egui::Ui) {
        let ready = self.state_loaded;
        widgets::card(ui, "Power", "Reduce CPU load and heat", |ui| {
            let busy = self.is_busy(Job::LowPower);
            if let Some(enable) = Self::toggle_row(
                ui,
                "Low Power Mode",
                "Custom power scheme: CPU capped at 60%/50%, turbo boost off, passive cooling. Off restores your previous plan.",
                self.state.low_power,
                busy,
                ready,
            ) {
                self.state.low_power = enable;
                self.start_job(Job::LowPower, move |log| {
                    if enable { power::enable_low_power(log) } else { power::disable_low_power(log) }
                });
            }
        });
    }

    fn tools_card(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "System Tools", "Quick launchers for built-in Windows utilities", |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("tools")
                    .width((ui.available_width() - 150.0).max(120.0))
                    .selected_text(TOOLS[self.tool_idx].label)
                    .show_ui(ui, |ui| {
                        for (i, tool) in TOOLS.iter().enumerate() {
                            ui.selectable_value(&mut self.tool_idx, i, tool.label).on_hover_text(tool.description);
                        }
                    });
                if widgets::primary_button(ui, "Open", true).clicked() {
                    let tool = &TOOLS[self.tool_idx];
                    match tool.launch() {
                        Ok(()) => self.logger.info(format!("Opened {}.", tool.label)),
                        Err(e) => self.logger.error(e),
                    }
                }
            });
            ui.label(RichText::new(TOOLS[self.tool_idx].description).small().weak());
        });
    }

    // ------------------------------------------------------------------
    // Windows (scan output, task list, confirmations)
    // ------------------------------------------------------------------

    fn scan_window(&mut self, ctx: &egui::Context) {
        let Some(scan) = &mut self.scan else { return };
        let mut open = true;
        let mut close_requested = false;
        egui::Window::new(format!("Scan Disk — {}", scan.title))
            .id(egui::Id::unique("scan_window"))
            .open(&mut open)
            .default_size([760.0, 480.0])
            .min_size([480.0, 260.0])
            .collapsible(false)
            .show(ctx, |ui| {
                // Status line
                ui.horizontal(|ui| match &scan.finished {
                    None => {
                        ui.add(egui::Spinner::new());
                        ui.label(format!("Running… {}s", scan.started.elapsed().as_secs()));
                    }
                    Some((Ok(code), took)) => {
                        let color = if *code == 0 { OK_GREEN } else { WARN_AMBER };
                        ui.colored_label(color, format!("Finished in {}s — exit code {code}: {}", took.as_secs(), disk::describe_exit(*code)));
                    }
                    Some((Err(e), _)) => {
                        ui.colored_label(ERR_RED, format!("Failed: {e}"));
                    }
                });
                ui.add_space(4.0);

                // Terminal view
                let bottom_bar = 36.0;
                egui::Frame::new()
                    .fill(Color32::from_rgb(12, 12, 14))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        egui::ScrollArea::both()
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .max_height(ui.available_height() - bottom_bar)
                            .show(ui, |ui| {
                                let term = Color32::from_rgb(210, 220, 210);
                                for line in &scan.lines {
                                    ui.label(RichText::new(line).monospace().color(term));
                                }
                                if let Some(p) = &scan.progress {
                                    ui.label(RichText::new(p).monospace().color(ACCENT));
                                }
                                if scan.lines.is_empty() && scan.progress.is_none() {
                                    ui.label(RichText::new("Waiting for chkdsk output…").monospace().weak());
                                }
                            });
                    });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if scan.finished.is_none() {
                        if ui.button("Cancel scan").clicked() {
                            if let Err(e) = processes::terminate(scan.pid) {
                                self.logger.error(format!("Could not stop chkdsk: {e}"));
                            } else {
                                self.logger.warn("chkdsk cancelled.");
                            }
                        }
                    } else if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                    if ui.button("Copy output").clicked() {
                        ui.ctx().copy_text(scan.lines.join("\n"));
                    }
                });
            });
        // Keep the elapsed-time counter ticking while the scan runs.
        if scan.finished.is_none() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if !open || close_requested {
            if scan.finished.is_none() {
                // Closing the window mid-scan stops chkdsk rather than orphaning it.
                let _ = processes::terminate(scan.pid);
                self.logger.warn("chkdsk cancelled.");
            }
            self.scan = None;
        }
    }

    fn task_window(&mut self, ctx: &egui::Context) {
        let Some(tw) = &mut self.tasks else { return };
        let mut open = true;
        let mut action: Option<ProcAction> = None;

        let snapshot: Vec<ProcInfo> = tw.monitor.snapshot.lock().map(|g| g.clone()).unwrap_or_default();
        let filter = tw.filter.to_lowercase();
        let mut rows: Vec<&ProcInfo> = snapshot
            .iter()
            .filter(|p| tw.show_background || p.kind == ProcKind::App)
            .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter) || p.pid.to_string() == filter)
            .collect();
        match tw.sort {
            SortBy::Name => rows.sort_by_key(|p| p.name.to_lowercase()),
            SortBy::Cpu => rows.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
            SortBy::Memory => rows.sort_by_key(|p| std::cmp::Reverse(p.memory)),
        }
        // Forget selections for processes that have exited.
        let alive: HashSet<u32> = snapshot.iter().map(|p| p.pid).collect();
        tw.selected.retain(|pid| alive.contains(pid));
        self.suspended.retain(|pid| alive.contains(pid));

        egui::Window::new("Optimize Task")
            .id(egui::Id::unique("task_window"))
            .open(&mut open)
            .default_size([820.0, 540.0])
            .min_size([600.0, 320.0])
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.add(egui::TextEdit::singleline(&mut tw.filter).hint_text("name or PID").desired_width(200.0));
                    ui.checkbox(&mut tw.show_background, "Show background processes");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} processes", snapshot.len())).weak());
                    });
                });
                ui.add_space(4.0);

                let selected_count = tw.selected.len();
                ui.horizontal(|ui| {
                    let any = selected_count > 0;
                    if widgets::primary_button(ui, "⚡ Optimize", any).on_hover_text("Trim memory and set Below Normal priority").clicked() {
                        action = Some(ProcAction::Optimize);
                    }
                    if ui.add_enabled(any, egui::Button::new("⏸ Suspend")).clicked() {
                        action = Some(ProcAction::Suspend);
                    }
                    if ui.add_enabled(any, egui::Button::new("▶ Resume")).clicked() {
                        action = Some(ProcAction::Resume);
                    }
                    if ui.add_enabled(any, egui::Button::new(RichText::new("✖ End task").color(ERR_RED))).clicked() {
                        action = Some(ProcAction::End);
                    }
                    if ui.add_enabled(any, egui::Button::new("Clear selection")).clicked() {
                        tw.selected.clear();
                    }
                    ui.label(RichText::new(format!("{selected_count} selected")).weak());
                });
                ui.separator();

                let row_h = 22.0;
                let header = |ui: &mut egui::Ui, text: &str, key: SortBy, sort: &mut SortBy| {
                    let label = if *sort == key { format!("{text} ▼") } else { text.to_string() };
                    if ui.add(egui::Button::new(RichText::new(label).strong()).frame(false)).clicked() {
                        *sort = key;
                    }
                };
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("proc_grid").striped(true).num_columns(7).spacing([14.0, 4.0]).min_row_height(row_h).show(ui, |ui| {
                        ui.label("");
                        header(ui, "Name", SortBy::Name, &mut tw.sort);
                        ui.label(RichText::new("PID").strong());
                        ui.label(RichText::new("Type").strong());
                        header(ui, "CPU", SortBy::Cpu, &mut tw.sort);
                        header(ui, "Memory", SortBy::Memory, &mut tw.sort);
                        ui.label(RichText::new("Status").strong());
                        ui.end_row();

                        for p in &rows {
                            let mut checked = tw.selected.contains(&p.pid);
                            if ui.checkbox(&mut checked, "").changed() {
                                if checked { tw.selected.insert(p.pid); } else { tw.selected.remove(&p.pid); }
                            }
                            let heavy = p.memory >= 1024 * 1024 * 1024 || p.cpu >= 15.0;
                            let name = RichText::new(&p.name);
                            ui.label(if heavy { name.color(ACCENT).strong() } else { name });
                            ui.label(RichText::new(p.pid.to_string()).monospace().weak());
                            ui.label(match p.kind { ProcKind::App => "App", ProcKind::Background => "Background" });
                            let cpu = RichText::new(format!("{:.1}%", p.cpu)).monospace();
                            ui.label(if p.cpu >= 15.0 { cpu.color(ACCENT) } else { cpu });
                            let mem = RichText::new(fmt_bytes(p.memory)).monospace();
                            ui.label(if p.memory >= 1024 * 1024 * 1024 { mem.color(ACCENT) } else { mem });
                            if self.suspended.contains(&p.pid) {
                                ui.colored_label(WARN_AMBER, "Suspended");
                            } else if p.protected {
                                ui.label(RichText::new("🔒 Protected").weak());
                            } else {
                                ui.label("");
                            }
                            ui.end_row();
                        }
                    });
                });
            });

        if let Some(action) = action {
            let targets: Vec<(u32, String, bool)> = snapshot
                .iter()
                .filter(|p| tw.selected.contains(&p.pid))
                .map(|p| (p.pid, p.name.clone(), p.protected))
                .collect();
            match action {
                ProcAction::End => {
                    let allowed: Vec<(u32, String)> = targets.iter().filter(|t| !t.2).map(|t| (t.0, t.1.clone())).collect();
                    if allowed.len() < targets.len() {
                        self.logger.warn("Protected system processes were skipped.");
                    }
                    if !allowed.is_empty() {
                        self.confirm_end = Some(allowed);
                    }
                }
                _ => self.run_proc_action(action, targets),
            }
        }
        if !open {
            // Dropping the TaskWindow stops its sampling thread.
            self.tasks = None;
        }
    }

    /// Applies a process action on a worker thread.
    fn run_proc_action(&mut self, action: ProcAction, targets: Vec<(u32, String, bool)>) {
        let mut work = Vec::new();
        for (pid, name, protected) in targets {
            // Only memory trimming is safe on protected processes.
            if protected && !matches!(action, ProcAction::Optimize) {
                self.logger.warn(format!("{name} ({pid}) is protected and was skipped."));
                continue;
            }
            match action {
                ProcAction::Suspend => {
                    self.suspended.insert(pid);
                }
                ProcAction::Resume | ProcAction::End => {
                    self.suspended.remove(&pid);
                }
                ProcAction::Optimize => {}
            }
            work.push((pid, name, protected));
        }
        if work.is_empty() {
            return;
        }
        if self.is_busy(Job::ProcessAction) {
            self.logger.warn("Another process action is still running; try again in a moment.");
            return;
        }
        self.start_job(Job::ProcessAction, move |log| {
            let mut ok = 0;
            for (pid, name, protected) in &work {
                let result = match action {
                    ProcAction::Optimize if *protected => ram::trim_process(*pid),
                    ProcAction::Optimize => processes::optimize(*pid),
                    ProcAction::Suspend => processes::suspend(*pid),
                    ProcAction::Resume => processes::resume(*pid),
                    ProcAction::End => processes::terminate(*pid),
                };
                match result {
                    Ok(()) => ok += 1,
                    Err(e) => log.warn(format!("{name} ({pid}): {e}")),
                }
            }
            log.success(format!("{} {ok} of {} process(es).", action.verb(), work.len()));
            Ok(())
        });
    }

    fn confirm_end_dialog(&mut self, ctx: &egui::Context) {
        let Some(targets) = &self.confirm_end else { return };
        let mut decision: Option<bool> = None;
        egui::Modal::new(egui::Id::unique("confirm_end")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("End these tasks?");
            ui.label("Unsaved work in these programs will be lost.");
            ui.add_space(6.0);
            for (pid, name) in targets.iter().take(12) {
                ui.label(format!("• {name} ({pid})"));
            }
            if targets.len() > 12 {
                ui.label(format!("…and {} more", targets.len() - 12));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("End task").color(Color32::WHITE)).fill(ERR_RED)).clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });
        match decision {
            Some(true) => {
                let targets = self.confirm_end.take().unwrap_or_default();
                if let Some(tw) = &mut self.tasks {
                    tw.selected.clear();
                }
                self.run_proc_action(ProcAction::End, targets.into_iter().map(|(p, n)| (p, n, false)).collect());
            }
            Some(false) => self.confirm_end = None,
            None => {}
        }
    }
}

impl eframe::App for RustCleanupApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Live memory gauge, refreshed once a second.
        if self.mem_checked.elapsed() >= Duration::from_secs(1) {
            self.mem = ram::mem_status();
            self.mem_checked = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(ctx.global_style().visuals.panel_fill).inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ui, |ui| self.header(ui));

        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(140.0)
            .min_size(80.0)
            .frame(egui::Frame::new().fill(ctx.global_style().visuals.extreme_bg_color).inner_margin(egui::Margin::symmetric(16, 8)))
            .show(ui, |ui| self.activity_log(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ctx.global_style().visuals.panel_fill).inner_margin(egui::Margin::symmetric(16, 12)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.columns(2, |cols| {
                        cols[0].spacing_mut().item_spacing.y = 12.0;
                        cols[1].spacing_mut().item_spacing.y = 12.0;
                        self.memory_card(&mut cols[0]);
                        self.disk_card(&mut cols[0]);
                        self.tasks_card(&mut cols[0]);
                        self.privacy_card(&mut cols[1]);
                        self.power_card(&mut cols[1]);
                        self.tools_card(&mut cols[1]);
                    });
                });
            });

        self.scan_window(&ctx);
        self.task_window(&ctx);
        self.confirm_end_dialog(&ctx);
    }
}

impl Drop for RustCleanupApp {
    /// Never leave processes frozen behind us: resume anything we suspended,
    /// and stop a chkdsk that is still running.
    fn drop(&mut self) {
        for pid in self.suspended.drain() {
            let _ = processes::resume(pid);
        }
        if let Some(scan) = &self.scan
            && scan.finished.is_none() {
                let _ = processes::terminate(scan.pid);
            }
    }
}

/// Dark theme with the rust-orange accent.
fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(24, 24, 28);
    visuals.window_fill = Color32::from_rgb(30, 30, 35);
    visuals.faint_bg_color = Color32::from_rgb(34, 34, 40);
    visuals.extreme_bg_color = Color32::from_rgb(17, 17, 20);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.6);
    visuals.hyperlink_color = ACCENT;
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        for (text_style, font) in style.text_styles.iter_mut() {
            match text_style {
                egui::TextStyle::Body | egui::TextStyle::Button => font.size = 14.5,
                egui::TextStyle::Small => font.size = 12.0,
                _ => {}
            }
        }
    });
}
