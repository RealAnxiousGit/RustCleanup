//! The Changelog window: the GitHub release notes of the running version.
//! It opens by itself on the first start after an update.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Margin, RichText};

use super::theme::{self, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{tr, trf};
use crate::sys::updater;

pub const WINDOW: &str = "changelog_window";

#[derive(Default)]
struct State {
    open: bool,
    /// Release notes and page, or why they could not be loaded.
    notes: Option<Result<(String, String), String>>,
}

pub struct Changelog {
    state: Arc<Mutex<State>>,
    jobs: Jobs,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl Changelog {
    pub fn new(jobs: Jobs) -> Self {
        Self {
            state: Arc::default(),
            jobs,
        }
    }

    pub fn open(&self, ctx: &egui::Context) {
        let mut state = lock(&self.state);
        if state.open {
            super::focus_window(ctx, WINDOW);
        }
        state.open = true;
        if !matches!(state.notes, Some(Ok(_))) {
            state.notes = None;
            drop(state);
            self.fetch(ctx);
        }
    }

    /// Downloads the notes on a worker thread.
    fn fetch(&self, ctx: &egui::Context) {
        let state = self.state.clone();
        let ctx = ctx.clone();
        self.jobs.start(Job::Changelog, move |_| {
            let result = updater::notes_for(env!("CARGO_PKG_VERSION"));
            lock(&state).notes = Some(result);
            ctx.request_repaint_of(super::window_id(WINDOW));
            // A failure is shown in the window, not in the activity log.
            Ok(())
        });
    }

    /// Keeps the window alive. Called on every pass of the main window.
    pub fn show(&self, ctx: &egui::Context) {
        if !lock(&self.state).open {
            return;
        }
        let state = self.state.clone();
        let jobs = self.jobs.clone();
        let refetch = Self {
            state: self.state.clone(),
            jobs: self.jobs.clone(),
        };
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(tr("Changelog"), [600.0, 640.0], [440.0, 360.0]),
            move |ui, _class| {
                let pal = theme::palette();
                let mut close = super::close_pressed(ui.ctx());
                let mut retry = false;
                let notes = lock(&state).notes.clone();

                egui::Panel::bottom("changelog_actions")
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::symmetric(22, 14)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if widgets::button(ui, tr("Close"), true).clicked() {
                                close = true;
                            }
                            if let Some(Ok((_, page))) = &notes
                                && !page.is_empty()
                            {
                                ui.add_space(6.0);
                                widgets::link(ui, &jobs.log, tr("Open the release page"), page);
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
                            RichText::new(trf(
                                "What's new in RustCleanup {0}",
                                &[&env!("CARGO_PKG_VERSION")],
                            ))
                            .family(semibold())
                            .size(20.0),
                        );
                        widgets::note(ui, &pal, tr("From this version's release on GitHub."));
                        ui.add_space(10.0);
                        match &notes {
                            None => {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                                    ui.label(tr("Loading the changelog."));
                                });
                            }
                            Some(Err(e)) => {
                                ui.label(
                                    RichText::new(trf("Could not load the changelog: {0}", &[e]))
                                        .color(pal.error),
                                );
                                ui.add_space(6.0);
                                if widgets::button(ui, tr("Try again"), !jobs.is_busy(Job::Changelog))
                                    .clicked()
                                {
                                    retry = true;
                                }
                            }
                            Some(Ok((markdown, _))) => {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| super::update::notes(ui, &pal, markdown));
                            }
                        }
                    });

                if retry {
                    lock(&state).notes = None;
                    refetch.fetch(ui.ctx());
                }
                if close {
                    lock(&state).open = false;
                    super::closed(ui.ctx());
                }
            },
        );
    }
}
