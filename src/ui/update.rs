//! The "new version available" window, shown when automatic updates are
//! off. Closing it just asks again on the next launch.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Margin, RichText, Ui};

use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::events::{AppEvent, Job, Jobs};
use crate::i18n::{tr, trf};
use crate::settings;
use crate::sys::updater::{self, Release};

pub const WINDOW: &str = "update_window";

pub struct UpdateWindow {
    release: Arc<Mutex<Option<Release>>>,
    jobs: Jobs,
}

fn lock(r: &Mutex<Option<Release>>) -> MutexGuard<'_, Option<Release>> {
    r.lock().unwrap_or_else(|e| e.into_inner())
}

impl UpdateWindow {
    pub fn new(jobs: Jobs) -> Self {
        Self {
            release: Arc::default(),
            jobs,
        }
    }

    pub fn open(&self, ctx: &egui::Context, release: Release) {
        let was_open = lock(&self.release).replace(release).is_some();
        if was_open {
            super::focus_window(ctx, WINDOW);
        }
    }

    /// Keeps the window alive while there is a release to offer.
    pub fn show(&self, ctx: &egui::Context) {
        if lock(&self.release).is_none() {
            return;
        }
        let release = self.release.clone();
        let jobs = self.jobs.clone();
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(tr("Update available"), [560.0, 560.0], [420.0, 340.0]),
            move |ui, _class| {
                let mut guard = lock(&release);
                let Some(r) = guard.clone() else { return };
                let installing = jobs.is_busy(Job::UpdateInstall);
                let pal = theme::palette();
                let mut close = !installing && super::close_pressed(ui.ctx());

                egui::Panel::bottom("update_actions")
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::symmetric(22, 14)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if widgets::primary_button(ui, &pal, tr("Update Now"), !installing)
                                .clicked()
                            {
                                let r = r.clone();
                                jobs.start(Job::UpdateInstall, move |log| {
                                    let exe = updater::install(&r, log)?;
                                    log.send(AppEvent::Restart(exe));
                                    Ok(())
                                });
                            }
                            if widgets::button(ui, tr("Never Update"), !installing)
                                .on_hover_text(tr(
                                    "Stop asking about new versions. You can turn this back on in Settings.",
                                ))
                                .clicked()
                            {
                                settings::update(|s| {
                                    s.update_notify = false;
                                    s.auto_update = false;
                                });
                                jobs.log.info(tr(
                                    "RustCleanup will not ask about new versions again. Turn \"Tell me about new versions\" back on in Settings to change this.",
                                ));
                                close = true;
                            }
                            if installing {
                                ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                                widgets::note(ui, &pal, tr("Downloading and installing."));
                            }
                        });
                    });

                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::same(22)),
                    )
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(trf("RustCleanup {0} is available", &[&r.version]))
                                .family(semibold())
                                .size(20.0),
                        );
                        widgets::note(
                            ui,
                            &pal,
                            trf("You have version {0}.", &[&updater::current_version()]),
                        );
                        if !r.page.is_empty() {
                            widgets::link(ui, &jobs.log, tr("Open the release page"), &r.page);
                        }
                        ui.add_space(8.0);
                        widgets::heading(ui, tr("What's new"));
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| notes(ui, &pal, &r.notes));
                    });

                if close {
                    *guard = None;
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

/// Draws GitHub release notes: headings, bullets and plain lines. Images,
/// links and emphasis markers are reduced to their text.
pub fn notes(ui: &mut Ui, pal: &Palette, markdown: &str) {
    let clean = |s: &str| s.replace("**", "").replace('`', "");
    for line in markdown.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            ui.add_space(4.0);
        } else if line.trim_start().starts_with("![") || line.trim_start().starts_with("<!--") {
            continue;
        } else if let Some(h) = line.strip_prefix('#') {
            ui.add_space(4.0);
            ui.label(
                RichText::new(clean(h.trim_start_matches('#').trim()))
                    .family(semibold())
                    .size(15.0),
            );
        } else if let Some(item) = line
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| line.trim_start().strip_prefix("* "))
        {
            let indent = (line.len() - line.trim_start().len()) as f32 * 6.0;
            ui.horizontal_wrapped(|ui| {
                ui.add_space(indent);
                ui.label(RichText::new("\u{2022}").color(pal.muted));
                ui.label(clean(item));
            });
        } else {
            ui.label(clean(line));
        }
    }
}
