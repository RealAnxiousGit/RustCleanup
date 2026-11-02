//! The main window: wires the sections, detached windows, tray icon and
//! theme together, and routes background events.
//!
//! The UI thread only draws and reads channels. Anything that could take
//! more than a few milliseconds runs on a worker thread (`Jobs::start`).
//!
//! This window's pass also runs while it is hidden in the tray (eframe
//! still repaints invisible windows), which is what keeps the detached
//! windows and notifications working without it.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, RichText, ViewportCommand};

use crate::background;
use crate::events::{AppEvent, Job, Jobs, Level, LogEntry, Logger};
use crate::i18n::{self, Lang, tr, trf};
use crate::settings::{self, FontChoice};
use crate::sys::privilege::is_elevated;
use crate::sys::shell::{self, WindowId};
use crate::sys::tray::{NotifyKind, Tray, TrayCommand};
use crate::ui::activity::ActivityLog;
use crate::ui::changelog::Changelog;
use crate::ui::credits::Credits;
use crate::ui::disk::DiskPanel;
use crate::ui::memory::MemoryPanel;
use crate::ui::settings::SettingsWindow;
use crate::ui::tasks::TaskList;
use crate::ui::theme::{self, Theme};
use crate::ui::tweaks::TweaksPanel;
use crate::ui::update::UpdateWindow;
use crate::ui::{self as views, widgets};

pub const MAIN_TITLE: &str = "RustCleanup";

/// Log entries that arrive close together become one notification: it is
/// shown this long after the last of them...
const NOTIFY_QUIET: Duration = Duration::from_millis(1200);
/// ...or this long after the first, whichever comes sooner.
const NOTIFY_MAX_WAIT: Duration = Duration::from_secs(4);
/// Windows cuts notification text off at 255 characters.
const NOTIFY_MAX_CHARS: usize = 255;

/// Log entries waiting to be shown as one notification.
#[derive(Default)]
struct PendingNotification {
    lines: Vec<String>,
    kind: Option<NotifyKind>,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl PendingNotification {
    fn push(&mut self, entry: &LogEntry) {
        let kind = match entry.level {
            Level::Info | Level::Success => NotifyKind::Info,
            Level::Warn => NotifyKind::Warning,
            Level::Error => NotifyKind::Error,
        };
        self.kind = self.kind.max(Some(kind));
        self.lines.push(entry.message.clone());
        let now = Instant::now();
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// The notification to show now, if it is time.
    fn take_due(&mut self) -> Option<(NotifyKind, String)> {
        let (first, last) = (self.first?, self.last?);
        if last.elapsed() < NOTIFY_QUIET && first.elapsed() < NOTIFY_MAX_WAIT {
            return None;
        }
        let kind = self.kind.unwrap_or(NotifyKind::Info);
        let mut body = self.lines.join("\n");
        if body.chars().count() > NOTIFY_MAX_CHARS {
            body = body.chars().take(NOTIFY_MAX_CHARS - 3).collect::<String>() + "...";
        }
        *self = Self::default();
        Some((kind, body))
    }
}

pub struct RustCleanupApp {
    rx: Receiver<AppEvent>,
    jobs: Jobs,
    /// `None` until the background check below answers.
    elevated: Option<bool>,
    elevation: Option<Receiver<bool>>,
    theme: Theme,

    activity: ActivityLog,
    memory: MemoryPanel,
    disk: DiskPanel,
    tweaks: TweaksPanel,
    tasks: TaskList,
    credits: Credits,
    changelog: Changelog,
    settings: SettingsWindow,
    update: UpdateWindow,

    tray: Option<Tray>,
    main_window: Option<WindowId>,
    /// Hidden to the tray (the close button was pressed, or it started there).
    in_tray: bool,
    minimized: bool,
    tray_hint_shown: bool,
    /// Frames left in which to hide the window again. eframe makes the root
    /// window visible after its first frame, even if it was built hidden.
    hide_frames: u8,
    pending: PendingNotification,
    /// Language and size the fonts and zoom were last set up for.
    applied_lang: Lang,
    applied_scale: u16,
    applied_font: FontChoice,
}

impl RustCleanupApp {
    pub fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Self {
        let ctx = &cc.egui_ctx;
        let s = settings::get();
        i18n::set_language(s.language);
        theme::install_fonts(ctx, s.language, s.font);
        ctx.set_zoom_factor(s.ui_scale as f32 / 100.0);

        let (tx, rx) = mpsc::channel();
        let log = Logger::new(tx, ctx.clone());
        let jobs = Jobs::new(log.clone());
        // Checked on a worker so the footer can show "checking" first.
        let (elevated_tx, elevation) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = elevated_tx.send(is_elevated());
            repaint.request_repaint();
        });

        // The tray forwards menu picks into the same event channel.
        let tray_log = log.clone();
        let tray = match Tray::start(move |cmd| tray_log.send(AppEvent::Tray(cmd))) {
            Ok(t) => Some(t),
            Err(e) => {
                log.warn(trf(
                    "No tray icon ({0}). Closing the window will quit RustCleanup.",
                    &[&e],
                ));
                None
            }
        };
        // Without a tray there would be no way back to a hidden window.
        let in_tray = start_hidden && tray.is_some();
        if start_hidden && !in_tray {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        }

        TweaksPanel::detect(&jobs);
        // Read before on_launch deletes the old exe an update leaves behind.
        let updated = settings::note_version(
            env!("CARGO_PKG_VERSION"),
            crate::sys::updater::leftover_exists(),
        );
        background::on_launch(&jobs);
        let changelog = Changelog::new(jobs.clone());
        if updated {
            // First start of a new version: show what changed, once.
            changelog.open(ctx);
        }

        Self {
            rx,
            elevated: None,
            elevation: Some(elevation),
            theme: Theme::load(),
            activity: ActivityLog::new(),
            memory: MemoryPanel::new(),
            disk: DiskPanel::new(jobs.clone()),
            tweaks: TweaksPanel::new(),
            tasks: TaskList::new(jobs.clone()),
            credits: Credits::new(log),
            changelog,
            settings: SettingsWindow::new(jobs.clone()),
            update: UpdateWindow::new(jobs.clone()),
            jobs,
            tray,
            main_window: None,
            in_tray,
            minimized: false,
            tray_hint_shown: in_tray,
            hide_frames: if in_tray { 2 } else { 0 },
            pending: PendingNotification::default(),
            applied_lang: s.language,
            applied_scale: s.ui_scale,
            applied_font: s.font,
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
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.in_tray = false;
        self.hide_frames = 0;
        ctx.request_repaint();
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.in_tray = true;
        if !self.tray_hint_shown {
            self.tray_hint_shown = true;
            // Shown as a notification too, since the window is now hidden.
            self.jobs.log.info(tr("RustCleanup is still running in the notification area. Right-click its icon for options, or choose Exit to quit."));
        }
    }

    /// Undoes anything temporary: frozen processes and a running chkdsk.
    fn clean_up(&mut self) {
        self.tasks.resume_all();
        self.disk.cancel();
    }

    /// Restores anything changed temporarily, then ends the process.
    fn exit(&mut self) -> ! {
        self.clean_up();
        if let Some(tray) = &mut self.tray {
            tray.remove();
        }
        std::process::exit(0);
    }

    /// Starts `exe` (a new version, or this one again) and quits. The new
    /// copy waits for this one to finish before taking over.
    fn restart(&mut self, exe: &Path) {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg(crate::RESTARTED_ARG);
        if self.in_tray {
            cmd.arg(crate::HIDDEN_ARG);
        }
        match cmd.spawn() {
            Ok(_) => self.exit(),
            Err(e) => self.jobs.log.error(trf(
                "Could not restart RustCleanup: {0}. Close it and open it again.",
                &[&e],
            )),
        }
    }

    fn handle_tray(&mut self, ctx: &egui::Context, cmd: TrayCommand) {
        match cmd {
            TrayCommand::OpenApp => self.show_from_tray(ctx),
            TrayCommand::CleanRam => MemoryPanel::start_clean(&self.jobs),
            // These open as windows of their own; the main window stays where it is.
            TrayCommand::OpenTaskList => self.tasks.open(ctx),
            TrayCommand::Settings => self.settings.open(ctx),
            TrayCommand::Credits => self.credits.open(ctx),
            TrayCommand::Exit => self.exit(),
        }
    }

    /// Drains worker and tray messages. Runs even while the window is hidden.
    fn pump_events(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.elevation
            && let Ok(elevated) = rx.try_recv()
        {
            self.elevation = None;
            self.elevated = Some(elevated);
            if !elevated {
                self.jobs.log.warn(tr("RustCleanup is not running as Administrator, so most changes will fail. Restart it and choose Yes on the UAC prompt."));
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            match event {
                AppEvent::Log(entry) => {
                    // While the window is out of sight, everything logged also
                    // appears as a notification.
                    if (self.in_tray || self.minimized)
                        && self.tray.is_some()
                        && settings::get().notifications
                    {
                        self.pending.push(&entry);
                    }
                    self.activity.push(entry);
                }
                AppEvent::State(state) => {
                    self.tweaks.state = state;
                    self.tweaks.loaded = true;
                }
                AppEvent::JobFinished(job) => match job {
                    // Switches always resync with the real system state,
                    // so a failed change never leaves one showing the wrong value.
                    Job::Copilot | Job::Telemetry | Job::Updates | Job::LowPower => {
                        TweaksPanel::detect(&self.jobs)
                    }
                    Job::CleanRam => self.memory.refresh(true),
                    _ => {}
                },
                AppEvent::Tray(cmd) => self.handle_tray(ctx, cmd),
                AppEvent::UpdateAvailable(release) => {
                    self.jobs
                        .log
                        .info(trf("RustCleanup {0} is available.", &[&release.version]));
                    self.update.open(ctx, release);
                }
                AppEvent::UpToDate { manual } => {
                    if manual {
                        self.jobs.log.success(tr("RustCleanup is up to date."));
                    }
                }
                AppEvent::Restart(exe) => self.restart(&exe),
            }
        }
        self.disk.pump();

        if let Some((kind, body)) = self.pending.take_due() {
            if let Some(tray) = &self.tray {
                tray.notify(kind, "RustCleanup", &body);
            }
        } else if self.pending.first.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Picks up changes made in the Settings window.
    fn apply_settings(&mut self, ctx: &egui::Context) {
        let s = settings::get();
        if s.language != self.applied_lang || s.font != self.applied_font {
            self.applied_lang = s.language;
            self.applied_font = s.font;
            i18n::set_language(s.language);
            // Japanese and Chinese share characters but not glyph styles,
            // so the font order follows the language.
            theme::install_fonts(ctx, s.language, s.font);
            views::repaint_windows(ctx);
        }
        if s.ui_scale != self.applied_scale {
            self.applied_scale = s.ui_scale;
            ctx.set_zoom_factor(s.ui_scale as f32 / 100.0);
            views::repaint_windows(ctx);
        }
        if self.theme.update(ctx) {
            views::repaint_windows(ctx);
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let pal = self.theme.palette;
        // Right to left: the theme button sits at the far right, then
        // Settings, Credits and Changelog. The admin status fills the left.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::theme_button(ui, &pal, self.theme.light).clicked() {
                self.theme.toggle();
            }
            if widgets::button(ui, tr("Settings"), true)
                .on_hover_text(tr("Language, theme, font, startup and updates"))
                .clicked()
            {
                self.settings.open(ui.ctx());
            }
            if widgets::button(ui, tr("Credits"), true)
                .on_hover_text(tr("Who made RustCleanup and what it is built with"))
                .clicked()
            {
                self.credits.open(ui.ctx());
            }
            if widgets::button(ui, tr("Changelog"), true)
                .on_hover_text(tr("What changed in this version"))
                .clicked()
            {
                self.changelog.open(ui.ctx());
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                self.admin_status(ui);
            });
        });
    }

    /// A macOS-style dot: green with admin rights, yellow while checking,
    /// red (plus a warning sign) without them.
    fn admin_status(&mut self, ui: &mut egui::Ui) {
        let pal = self.theme.palette;
        match self.elevated {
            Some(true) => {
                widgets::status_dot(ui, pal.ok, tr("Admin Privileges"));
                widgets::note(ui, &pal, tr("Running as Administrator"));
            }
            None => {
                widgets::status_dot(ui, pal.warn, tr("Checking for Admin..."));
                widgets::note(ui, &pal, tr("Checking for Admin..."));
            }
            Some(false) => {
                widgets::status_dot(ui, pal.error, tr("Standard Privileges"));
                widgets::warning_sign(
                    ui,
                    pal.error,
                    tr("Not all features will work without admin rights. Close RustCleanup, open it again and choose Yes on the UAC prompt."),
                );
                ui.label(
                    RichText::new(tr("Not running as Administrator"))
                        .size(12.5)
                        .color(pal.error),
                );
            }
        }
        if self.jobs.any_visible_busy() {
            ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
        }
    }

    fn main_panels(&mut self, ui: &mut egui::Ui) {
        let pal = self.theme.palette;
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
                    .inner_margin(Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| self.footer(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 14,
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
                                    self.memory.ui(ui, &pal, &self.jobs);
                                    ui.add_space(22.0);
                                    self.disk.ui(ui, &pal);
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
                                    self.tweaks.settings_ui(ui, &pal, &self.jobs);
                                    ui.add_space(22.0);
                                    self.tweaks.tools_ui(ui, &pal, &self.jobs);
                                },
                            );
                        });
                    });
            });
    }
}

impl eframe::App for RustCleanupApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.hide_frames > 0 {
            self.hide_frames -= 1;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.request_repaint();
        }
        self.apply_settings(&ctx);
        if self.main_window.is_none() {
            self.main_window = shell::find_own_window(MAIN_TITLE);
        }

        // Close button: hide to the tray instead of quitting, if that is
        // the setting and there is a tray to come back from.
        let (close, minimized) =
            ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized));
        self.minimized = minimized.unwrap_or(false);
        if close {
            if self.tray.is_some() && settings::get().close_to_tray {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_to_tray(&ctx);
            } else {
                self.clean_up();
            }
        }

        // Nothing to draw while hidden; the detached windows below still
        // need this pass to stay open.
        if !self.in_tray {
            // The memory figures are live: refresh once a second, and only then.
            self.memory.refresh(false);
            ctx.request_repaint_after(Duration::from_secs(1));
            self.main_panels(ui);
        }

        self.disk.show(&ctx);
        self.tasks.show(&ctx);
        self.settings.show(&ctx);
        self.credits.show(&ctx);
        self.changelog.show(&ctx);
        self.update.show(&ctx);
    }
}

impl Drop for RustCleanupApp {
    /// Never leave processes frozen behind us, or chkdsk running.
    fn drop(&mut self) {
        self.clean_up();
    }
}
