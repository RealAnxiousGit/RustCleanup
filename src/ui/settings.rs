//! The Settings window.
//!
//! Every change is saved immediately (see `crate::settings`), so there is
//! no Apply button. The main window picks up language, size and theme
//! changes on its next pass.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Margin, RichText, Ui, ViewportId};

use super::flags;
use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::background;
use crate::events::{AppEvent, Job, Jobs};
use crate::i18n::{self, Lang, tr, trf};
use crate::settings::{self, AUTO_CLEAN_RANGE, Settings, ThemeMode, UI_SCALES};
use crate::sys::format::fmt_bytes;
use crate::sys::{memory, startup, updater};

pub const WINDOW: &str = "settings_window";

pub struct SettingsWindow {
    open: Arc<AtomicBool>,
    jobs: Jobs,
    /// Renderer this process started with; changing it needs a restart.
    hardware_ui_at_launch: bool,
}

impl SettingsWindow {
    pub fn new(jobs: Jobs) -> Self {
        Self {
            open: Arc::default(),
            jobs,
            hardware_ui_at_launch: settings::get().hardware_ui,
        }
    }

    pub fn open(&self, ctx: &egui::Context) {
        if self.open.swap(true, Ordering::Relaxed) {
            super::focus_window(ctx, WINDOW);
        }
    }

    /// Keeps the window alive. Called on every pass of the main window.
    pub fn show(&self, ctx: &egui::Context) {
        if !self.open.load(Ordering::Relaxed) {
            return;
        }
        let open = self.open.clone();
        let jobs = self.jobs.clone();
        let launched_hw = self.hardware_ui_at_launch;
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(tr("Settings"), [560.0, 760.0], [440.0, 420.0]),
            move |ui, _class| {
                let pal = theme::palette();
                let before = settings::get();
                let mut close = super::close_pressed(ui.ctx());
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::same(22)),
                    )
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                body(ui, &pal, &jobs, before, launched_hw);
                                ui.add_space(10.0);
                                if widgets::button(ui, tr("Close"), true).clicked() {
                                    close = true;
                                }
                            });
                    });
                if settings::get() != before {
                    // Language, size and theme are applied by the main window.
                    ui.ctx().request_repaint_of(ViewportId::ROOT);
                }
                if close {
                    open.store(false, Ordering::Relaxed);
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    widgets::heading(ui, title);
    ui.separator();
}

/// A label on the left and a control on the right.
fn labeled(ui: &mut Ui, pal: &Palette, title: &str, detail: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() - 200.0).max(140.0);
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.label(RichText::new(title).family(semibold()));
            if !detail.is_empty() {
                widgets::note(ui, pal, detail);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

fn body(ui: &mut Ui, pal: &Palette, jobs: &Jobs, s: Settings, launched_hw: bool) {
    language_picker(ui, pal, s.language);

    section(ui, tr("General"));
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Start minimized"),
        tr("Start in the notification area instead of opening the window."),
        s.start_minimized,
        true,
    ) {
        settings::update(|s| s.start_minimized = on);
    }
    let startup_busy = jobs.is_busy(Job::Startup);
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Open on startup"),
        tr("Open RustCleanup when you sign in to Windows, without a UAC prompt."),
        s.open_on_startup,
        !startup_busy,
    ) {
        settings::update(|s| s.open_on_startup = on);
        jobs.start(Job::Startup, move |log| {
            startup::set_enabled(log, on).inspect_err(|_| {
                // Show the real state again if the task couldn't be changed.
                settings::update(|s| s.open_on_startup = !on);
            })
        });
    }
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Close to the notification area"),
        tr("The close button hides the window and RustCleanup keeps running. Off: it quits."),
        s.close_to_tray,
        true,
    ) {
        settings::update(|s| s.close_to_tray = on);
    }
    labeled(ui, pal, tr("Theme"), "", |ui| {
        egui::ComboBox::from_id_salt("theme_mode")
            .width(170.0)
            .selected_text(s.theme.label())
            .show_ui(ui, |ui| {
                for mode in ThemeMode::ALL {
                    if ui.selectable_label(s.theme == mode, mode.label()).clicked() {
                        settings::update(|s| s.theme = mode);
                    }
                }
            });
    });
    labeled(
        ui,
        pal,
        tr("Interface size"),
        tr("Makes text and controls larger or smaller in every window."),
        |ui| {
            egui::ComboBox::from_id_salt("ui_scale")
                .width(170.0)
                .selected_text(format!("{}%", s.ui_scale))
                .show_ui(ui, |ui| {
                    for scale in UI_SCALES {
                        if ui
                            .selectable_label(s.ui_scale == scale, format!("{scale}%"))
                            .clicked()
                        {
                            settings::update(|s| s.ui_scale = scale);
                        }
                    }
                });
        },
    );

    section(ui, tr("Notifications"));
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Notifications"),
        tr(
            "While RustCleanup is minimized or in the notification area, everything it logs also appears as a Windows notification.",
        ),
        s.notifications,
        true,
    ) {
        settings::update(|s| s.notifications = on);
    }

    section(ui, tr("Updates"));
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Automatically check for updates"),
        tr(
            "On: when RustCleanup starts, a new release on GitHub is downloaded, checked and installed by itself. Off: you are asked first, with the changelog.",
        ),
        s.auto_update,
        true,
    ) {
        settings::update(|s| s.auto_update = on);
    }
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Tell me about new versions"),
        tr(
            "Shows the new release and its changelog when RustCleanup starts. \"Never Update\" turns this off.",
        ),
        s.update_notify || s.auto_update,
        !s.auto_update,
    ) {
        settings::update(|s| s.update_notify = on);
    }
    ui.horizontal(|ui| {
        let checking = jobs.is_busy(Job::UpdateCheck) || jobs.is_busy(Job::UpdateInstall);
        if widgets::button(ui, tr("Check for updates now"), !checking).clicked() {
            jobs.start(Job::UpdateCheck, |log| background::check_updates(log, true));
        }
        if checking {
            ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
        }
        widgets::note(
            ui,
            pal,
            trf("You have version {0}.", &[&updater::current_version()]),
        );
    });

    section(ui, tr("Memory"));
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Auto-Clean RAM"),
        &trf(
            "Runs Clean RAM by itself when physical memory use reaches the amount below. It checks every 5 seconds and runs at most once every {0} minutes.",
            &[&(background::AUTO_CLEAN_COOLDOWN.as_secs() / 60)],
        ),
        s.auto_clean,
        true,
    ) {
        settings::update(|s| s.auto_clean = on);
    }
    let usable = memory::snapshot().physical.map(|p| p.usable).unwrap_or(0);
    let mut percent = s.auto_clean_percent;
    labeled(
        ui,
        pal,
        tr("Auto-Clean RAM amount"),
        &if usable > 0 {
            trf(
                "Clean when more than {0} of {1} is in use.",
                &[
                    &fmt_bytes(usable / 100 * percent as u64),
                    &fmt_bytes(usable),
                ],
            )
        } else {
            String::new()
        },
        |ui| {
            ui.add_enabled_ui(s.auto_clean, |ui| {
                ui.spacing_mut().slider_width = 130.0;
                let slider = egui::Slider::new(&mut percent, AUTO_CLEAN_RANGE).suffix("%");
                if ui.add(slider).changed() {
                    settings::update(|s| s.auto_clean_percent = percent);
                }
            });
        },
    );
    let mut purge = s.purge_standby;
    if ui
        .checkbox(&mut purge, tr("Also purge the standby list (file cache)"))
        .changed()
    {
        settings::update(|s| s.purge_standby = purge);
    }

    section(ui, tr("Performance"));
    if let Some(on) = widgets::switch_row(
        ui,
        pal,
        tr("Hardware-accelerated UI"),
        tr(
            "Draws the interface with Direct3D 12 instead of OpenGL. It can be smoother on older or integrated graphics, but uses about three times as much memory.",
        ),
        s.hardware_ui,
        true,
    ) {
        settings::update(|s| s.hardware_ui = on);
    }
    if s.hardware_ui != launched_hw {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Restart RustCleanup to apply this.")).color(pal.warn));
            if widgets::button(ui, tr("Restart now"), true).clicked()
                && let Ok(exe) = std::env::current_exe()
            {
                jobs.log.send(AppEvent::Restart(exe));
            }
        });
    }

    ui.add_space(14.0);
    ui.separator();
    if widgets::button(ui, tr("Reset all settings"), true)
        .on_hover_text(tr(
            "Every setting goes back to its default. Open on startup is kept.",
        ))
        .clicked()
    {
        settings::reset();
        jobs.log.info(tr("Settings were reset to their defaults."));
    }
}

fn language_picker(ui: &mut Ui, pal: &Palette, current: Lang) {
    // The English word stays next to the translation, so someone who picked
    // a language they can't read can still find the way back.
    let title = if current == Lang::En {
        tr("Language").to_string()
    } else {
        format!("{} (Language)", tr("Language"))
    };
    labeled(ui, pal, &title, "", |ui| {
        egui::ComboBox::from_id_salt("language")
            .width(170.0)
            .selected_text(current.native_name())
            .show_ui(ui, |ui| {
                for lang in Lang::ALL {
                    let row = ui.horizontal(|ui| {
                        flags::flag(ui, lang);
                        ui.selectable_label(lang == current, lang.native_name())
                    });
                    if row.inner.clicked() && lang != current {
                        settings::update(|s| s.language = lang);
                        i18n::set_language(lang);
                    }
                }
            });
        flags::flag(ui, current);
    });
}
