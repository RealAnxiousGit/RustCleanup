//! The Changelog window: the GitHub release notes of the running version.
//! It opens by itself on the first start after an update.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, RichText, Sense, Shape, Stroke, Ui};

use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{tr, trf};
use crate::sys::{shell, updater};

pub const WINDOW: &str = "changelog_window";
/// Still frame of the promotional trailer, shown where the notes embed it.
const TRAILER_THUMB: &[u8] = include_bytes!("../../assets/trailer-thumb.png");

#[derive(Default)]
struct State {
    open: bool,
    /// Release notes and page, or why they could not be loaded.
    notes: Option<Result<(String, String), String>>,
    thumb: Option<egui::TextureHandle>,
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
                let mut guard = lock(&state);
                let notes = guard.notes.clone();
                let thumb = guard
                    .thumb
                    .get_or_insert_with(|| {
                        let icon = eframe::icon_data::from_png_bytes(TRAILER_THUMB)
                            .unwrap_or_default();
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [icon.width as usize, icon.height as usize],
                            &icon.rgba,
                        );
                        ui.ctx()
                            .load_texture("trailer_thumb", image, egui::TextureOptions::LINEAR)
                    })
                    .clone();
                drop(guard);

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
                            Some(Ok((markdown, page))) => {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        super::update::notes(ui, &pal, markdown, &mut |ui, url| {
                                            trailer_card(ui, &pal, &jobs, &thumb, url, page);
                                        });
                                    });
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

/// A video in the notes: its still frame with a play button. Clicking it
/// plays the video on the release page, in the browser.
fn trailer_card(
    ui: &mut Ui,
    pal: &Palette,
    jobs: &Jobs,
    thumb: &egui::TextureHandle,
    url: &str,
    page: &str,
) {
    let width = ui.available_width().min(480.0);
    let size = egui::vec2(width, width * 9.0 / 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tr("Play the trailer in your browser"));
    let hover = widgets::fade(ui, response.id, response.hovered());
    let radius = CornerRadius::same(8);
    ui.painter().add(
        egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(thumb.id(), egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
    );
    ui.painter().rect_filled(
        rect,
        radius,
        Color32::from_black_alpha((40.0 + 30.0 * hover) as u8),
    );
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, pal.border),
        egui::StrokeKind::Inside,
    );
    let c = rect.center();
    let r = 28.0 + 3.0 * hover;
    ui.painter()
        .circle_filled(c, r, widgets::blend(pal.accent, pal.accent_hover, hover));
    let tri = vec![
        c + egui::vec2(-r * 0.32, -r * 0.45),
        c + egui::vec2(r * 0.5, 0.0),
        c + egui::vec2(-r * 0.32, r * 0.45),
    ];
    ui.painter()
        .add(Shape::convex_polygon(tri, pal.on_accent, Stroke::NONE));
    ui.painter().text(
        rect.left_bottom() + egui::vec2(12.0, -12.0),
        egui::Align2::LEFT_BOTTOM,
        tr("Watch the trailer"),
        egui::FontId::new(15.0, semibold()),
        Color32::WHITE,
    );
    if response.clicked() {
        // The release page plays the video inline; the bare file would download.
        let target = if page.is_empty() { url } else { page };
        if let Err(e) = shell::open_unelevated(target) {
            jobs.log.error(e);
        }
    }
    ui.add_space(8.0);
}
