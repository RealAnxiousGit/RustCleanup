//! The main window: wires the sections, detached windows, tray icon and
//! theme together, and routes background events.
//!
//! The UI thread only draws and reads channels. Anything that could take
//! more than a few milliseconds runs on a worker thread (`Jobs::start`).

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui::{self, Align, Layout, Margin, RichText, ViewportCommand};

use crate::events::{AppEvent, Job, Jobs, Level, Logger};
use crate::sys::privilege::is_elevated;
use crate::sys::shell::{self, WindowId};
use crate::sys::tray::{NotifyKind, Tray, TrayCommand};
use crate::ui::activity::ActivityLog;
use crate::ui::credits::Credits;
use crate::ui::disk::DiskPanel;
use crate::ui::memory::MemoryPanel;
use crate::ui::tasks::TaskList;
use crate::ui::theme::{self, Theme, semibold};
use crate::ui::tweaks::TweaksPanel;
use crate::ui::widgets;

pub const MAIN_TITLE: &str = "RustCleanup";

pub struct RustCleanupApp {
    rx: Receiver<AppEvent>,
    jobs: Jobs,
    elevated: bool,
    theme: Theme,

    activity: ActivityLog,
    memory: MemoryPanel,
    disk: DiskPanel,
    tweaks: TweaksPanel,
    tasks: TaskList,
    credits: Credits,

    tray: Option<Tray>,
    main_window: Option<WindowId>,
    /// Hidden to the tray (the close button was pressed).
    in_tray: bool,
    minimized: bool,
    tray_hint_shown: bool,
}

impl RustCleanupApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);

        let (tx, rx) = mpsc::channel();
        let log = Logger::new(tx, cc.egui_ctx.clone());
        let mut jobs = Jobs::new(log.clone());
        let elevated = is_elevated();

        // The tray forwards menu picks into the same event channel.
        let tray_log = log.clone();
        let tray = match Tray::start(move |cmd| tray_log.send(AppEvent::Tray(cmd))) {
            Ok(t) => Some(t),
            Err(e) => {
                log.warn(format!(
                    "No tray icon ({e}). Closing the window will quit RustCleanup."
                ));
                None
            }
        };

        if !elevated {
            log.warn("RustCleanup is not running as Administrator, so most changes will fail. Restart it and choose Yes on the UAC prompt.");
        }
        TweaksPanel::detect(&mut jobs);

        Self {
            rx,
            jobs,
            elevated,
            theme: Theme::load(),
            activity: ActivityLog::new(),
            memory: MemoryPanel::new(),
            disk: DiskPanel::new(),
            tweaks: TweaksPanel::new(),
            tasks: TaskList::new(),
            credits: Credits::new(),
            tray,
            main_window: None,
            in_tray: false,
            minimized: false,
            tray_hint_shown: false,
        }
    }

    fn show_from_tray(&mut self, ctx: &egui::Context) {
        if self.main_window.is_none() {
            self.main_window = shell::find_own_window(MAIN_TITLE);
        }
        if let Some(id) = self.main_window {
            shell::show_window(id);
        }
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        self.in_tray = false;
        ctx.request_repaint();
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.in_tray = true;
        if !self.tray_hint_shown {
            self.tray_hint_shown = true;
            let msg = "RustCleanup is still running in the notification area. Right-click its icon for options, or choose Exit to quit.";
            self.jobs.log.info(msg);
            if let Some(tray) = &self.tray {
                tray.notify(NotifyKind::Info, "RustCleanup", msg);
            }
        }
    }

    /// Restores anything changed temporarily, then ends the process.
    fn exit(&mut self) -> ! {
        self.tasks.resume_all();
        self.disk.cancel(&self.jobs);
        if let Some(tray) = &mut self.tray {
            tray.remove();
        }
        std::process::exit(0);
    }

    fn handle_tray(&mut self, ctx: &egui::Context, cmd: TrayCommand) {
        match cmd {
            TrayCommand::OpenApp => self.show_from_tray(ctx),
            TrayCommand::CleanRam => self.memory.start_clean(&mut self.jobs),
            TrayCommand::OpenTaskList => {
                self.show_from_tray(ctx);
                self.tasks.open(ctx);
            }
            TrayCommand::Credits => {
                self.show_from_tray(ctx);
                self.credits.open(ctx);
            }
            TrayCommand::Exit => self.exit(),
        }
    }

    /// Drains worker and tray messages. Runs even while the window is hidden.
    fn pump_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                AppEvent::Log(entry) => {
                    // While the window is out of sight, results also arrive as notifications.
                    if self.in_tray || self.minimized {
                        let kind = match entry.level {
                            Level::Info => None,
                            Level::Success => Some(NotifyKind::Info),
                            Level::Warn => Some(NotifyKind::Warning),
                            Level::Error => Some(NotifyKind::Error),
                        };
                        if let (Some(tray), Some(kind)) = (&self.tray, kind) {
                            tray.notify(kind, "RustCleanup", &entry.message);
                        }
                    }
                    self.activity.push(entry);
                }
                AppEvent::State(state) => {
                    self.tweaks.state = state;
                    self.tweaks.loaded = true;
                }
                AppEvent::JobFinished(job) => {
                    self.jobs.finished(job);
                    match job {
                        // Switches always resync with the real system state,
                        // so a failed change never leaves one showing the wrong value.
                        Job::Copilot | Job::Telemetry | Job::Updates | Job::LowPower => {
                            TweaksPanel::detect(&mut self.jobs)
                        }
                        Job::CleanRam => self.memory.refresh(true),
                        _ => {}
                    }
                }
                AppEvent::Tray(cmd) => self.handle_tray(ctx, cmd),
            }
        }
        self.disk.pump(&self.jobs);
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let pal = self.theme.palette;
        ui.horizontal(|ui| {
            ui.label(RichText::new("RustCleanup").family(semibold()).size(21.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.elevated {
                    widgets::note(ui, &pal, "Running as Administrator");
                } else {
                    ui.label(RichText::new("Not running as Administrator").color(pal.error));
                }
                if self.jobs.any_visible_busy() {
                    ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                }
            });
        });
        if !self.elevated {
            ui.label(
                RichText::new("Changes need admin rights. Close RustCleanup, open it again and choose Yes on the UAC prompt.")
                    .color(pal.error),
            );
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let pal = self.theme.palette;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::theme_button(ui, &pal, self.theme.light).clicked() {
                self.theme.toggle();
            }
            if widgets::button(ui, "Credits", true).clicked() {
                self.credits.open(ui.ctx());
            }
        });
    }
}

impl eframe::App for RustCleanupApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.theme.update(&ctx);
        let pal = self.theme.palette;
        if self.main_window.is_none() {
            self.main_window = shell::find_own_window(MAIN_TITLE);
        }

        // Close button: hide to the tray instead of quitting (if there is a tray).
        let (close, minimized) =
            ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized));
        self.minimized = minimized.unwrap_or(false);
        if close {
            if self.tray.is_some() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_to_tray(&ctx);
            } else {
                self.tasks.resume_all();
                self.disk.cancel(&self.jobs);
            }
        }

        // The memory figures are live: refresh once a second, and only then.
        self.memory.refresh(false);
        ctx.request_repaint_after(Duration::from_secs(1));

        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 14,
                bottom: 8,
            }))
            .show(ui, |ui| self.header(ui));

        egui::Panel::bottom("activity")
            .resizable(true)
            .default_size(150.0)
            .min_size(80.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.surface)
                    .inner_margin(Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| self.activity.ui(ui, &pal));

        // Added after the activity panel, so it sits just above it.
        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(Margin::symmetric(20, 6)),
            )
            .show(ui, |ui| self.footer(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 6,
                bottom: 6,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let gap = 28.0;
                        let total = ui.available_width();
                        // Two columns of unequal width: the memory side holds the main action.
                        let left_w = ((total - gap) * 0.56).floor();
                        let right_w = total - gap - left_w;
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            ui.allocate_ui_with_layout(
                                egui::vec2(left_w, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(left_w);
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    self.memory.ui(ui, &pal, &mut self.jobs);
                                    ui.add_space(22.0);
                                    self.disk.ui(ui, &pal, &mut self.jobs);
                                    ui.add_space(22.0);
                                    self.tasks.section_ui(ui, &pal);
                                },
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(right_w, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(right_w);
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    self.tweaks.settings_ui(ui, &pal, &mut self.jobs);
                                    ui.add_space(22.0);
                                    self.tweaks.tools_ui(ui, &pal, &mut self.jobs);
                                },
                            );
                        });
                    });
            });

        self.disk.window(&ctx, &self.theme, &self.jobs);
        self.tasks.window(&ctx, &self.theme, &mut self.jobs);
        self.credits.window(&ctx, &self.theme, &self.jobs.log);
    }
}

impl Drop for RustCleanupApp {
    /// Never leave processes frozen behind us, or chkdsk running.
    fn drop(&mut self) {
        self.tasks.resume_all();
        self.disk.cancel(&self.jobs);
    }
}
