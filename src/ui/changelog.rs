//! The Changelog window: the GitHub release notes of the running version.
//! It opens by itself on the first start after an update.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke, Ui};

use super::theme::{self, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{tr, trf};
use crate::sys::updater;
use crate::sys::video::{self, Phase, Player};

pub const WINDOW: &str = "changelog_window";

#[derive(Default)]
struct State {
    open: bool,
    /// The release, or why it could not be loaded.
    notes: Option<Result<updater::ReleaseNotes, String>>,
    /// The What's New video, once the play button was pressed.
    player: Option<Player>,
    picture: Option<egui::TextureHandle>,
}

/// The video a release links with a hidden line in its notes:
/// `<!-- video: https://.../file.mp4 | Title -->`.
fn video_of(markdown: &str) -> Option<(&str, &str)> {
    markdown.lines().find_map(|line| {
        let inner = line
            .trim()
            .strip_prefix("<!--")?
            .strip_suffix("-->")?
            .trim()
            .strip_prefix("video:")?;
        let (url, title) = inner.split_once('|').unwrap_or((inner, ""));
        let url = url.trim();
        url.starts_with("https://").then_some((url, title.trim()))
    })
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
                            if let Some(Ok(release)) = &notes
                                && !release.page.is_empty()
                            {
                                ui.add_space(6.0);
                                widgets::link(
                                    ui,
                                    &jobs.log,
                                    tr("Open the release page"),
                                    &release.page,
                                );
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
                        if let Some(date) =
                            notes.as_ref().and_then(|n| n.as_ref().ok()?.published.as_deref())
                        {
                            ui.add_space(2.0);
                            ui.label(RichText::new(date).color(pal.muted).size(14.0));
                            ui.add_space(8.0);
                        }
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
                            Some(Ok(release)) => {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        if let Some((url, title)) = video_of(&release.markdown) {
                                            video_card(ui, &pal, &state, &jobs, url, title);
                                            ui.add_space(14.0);
                                        }
                                        super::update::notes(ui, &pal, &release.markdown)
                                    });
                            }
                        }
                    });

                if retry {
                    lock(&state).notes = None;
                    refetch.fetch(ui.ctx());
                }
                if close {
                    let mut s = lock(&state);
                    s.open = false;
                    // Stops the sound and frees the decoder with the window.
                    s.player = None;
                    s.picture = None;
                    drop(s);
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

/// The What's New video: a play button until it is pressed, then the
/// picture with play, a seek bar, the time and mute under it.
fn video_card(
    ui: &mut Ui,
    pal: &theme::Palette,
    state: &Mutex<State>,
    jobs: &Jobs,
    url: &str,
    title: &str,
) {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, width * 9.0 / 16.0), Sense::click());
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(8);
    painter.rect_filled(rect, radius, Color32::from_rgb(0x10, 0x10, 0x12));

    let mut s = lock(state);
    // Newest picture to the GPU.
    if let Some(frame) = s.player.as_ref().and_then(Player::take_frame) {
        let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
        match &mut s.picture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                s.picture =
                    Some(ui.ctx().load_texture("whats_new_video", image, egui::TextureOptions::LINEAR))
            }
        }
    }
    let status = s.player.as_ref().map(Player::status);
    if let Some(texture) = &s.picture {
        // Letterboxed to keep the video's shape.
        let size = texture.size_vec2();
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let shown = egui::Rect::from_center_size(rect.center(), size * scale);
        painter.image(
            texture.id(),
            shown,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let phase = status.as_ref().map(|st| st.phase.clone());
    let center = rect.center();
    let overlay = |label: Option<String>| {
        // A round play button, with an optional line under it.
        let hovered = response.hovered();
        let fill = if hovered { pal.accent_hover } else { pal.accent };
        painter.circle_filled(center, 30.0, fill);
        let r = 12.0;
        let points = vec![
            center + egui::vec2(-r * 0.6, -r),
            center + egui::vec2(-r * 0.6, r),
            center + egui::vec2(r, 0.0),
        ];
        painter.add(egui::Shape::convex_polygon(points, pal.on_accent, Stroke::NONE));
        if let Some(label) = label {
            painter.text(
                center + egui::vec2(0.0, 50.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
    };
    match &phase {
        None => {
            overlay(Some(if title.is_empty() {
                tr("Watch what's new").to_string()
            } else {
                title.to_string()
            }));
        }
        Some(Phase::Downloading(p)) => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                trf("Downloading the video, {0}%", &[p]),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        Some(Phase::Loading) => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                tr("Opening the video."),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        Some(Phase::Paused) => overlay(None),
        Some(Phase::Ended) => overlay(Some(tr("Watch again").to_string())),
        Some(Phase::Playing) => {}
        Some(Phase::Failed(e)) => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                e,
                egui::FontId::proportional(13.0),
                pal.error,
            );
        }
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);

    // Clicking the picture plays, pauses or starts over.
    let toggle = |s: &mut State, ui: &Ui| match s.player.as_ref().map(|p| p.status().phase) {
        None | Some(Phase::Failed(_)) => {
            let ctx = ui.ctx().clone();
            let viewport = super::window_id(WINDOW);
            s.picture = None;
            s.player = Some(Player::start(url, move || ctx.request_repaint_of(viewport)));
        }
        Some(Phase::Playing) => s.player.as_ref().unwrap().pause(),
        Some(_) => s.player.as_ref().unwrap().play(),
    };
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    // Named for screen readers, which cannot see the painted play button.
    let name = match &phase {
        Some(Phase::Playing) => tr("Pause"),
        _ => tr("Play"),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    if response.clicked() {
        toggle(&mut s, ui);
    }

    let Some(status) = status else {
        return;
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = match status.phase {
            Phase::Playing => tr("Pause"),
            Phase::Ended => tr("Watch again"),
            _ => tr("Play"),
        };
        let ready = !matches!(status.phase, Phase::Downloading(_) | Phase::Loading);
        if widgets::button(ui, label, ready).clicked() {
            toggle(&mut s, ui);
        }
        let mute = if status.muted { tr("Unmute") } else { tr("Mute") };
        if widgets::button(ui, mute, ready).clicked()
            && let Some(player) = &s.player
        {
            player.set_muted(!status.muted);
        }
        ui.label(
            RichText::new(format!(
                "{} / {}",
                video::clock(status.position),
                video::clock(status.duration)
            ))
            .color(pal.muted),
        );
        if status.duration > 0.0 {
            let mut at = status.position;
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(60.0);
            let seek = ui.add(
                egui::Slider::new(&mut at, 0.0..=status.duration)
                    .show_value(false)
                    .trailing_fill(true),
            );
            if seek.changed()
                && let Some(player) = &s.player
            {
                player.seek(at);
            }
        }
    });
    if matches!(status.phase, Phase::Failed(_)) {
        drop(s);
        widgets::link(ui, &jobs.log, tr("Open the video in your browser"), url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_video_line() {
        let notes = "Intro\n<!-- video: https://example.com/v.mp4 | What's New -->\n- item";
        assert_eq!(video_of(notes), Some(("https://example.com/v.mp4", "What's New")));
        assert_eq!(
            video_of("<!-- video: https://example.com/v.mp4 -->"),
            Some(("https://example.com/v.mp4", ""))
        );
        assert_eq!(video_of("<!-- video: http://insecure/v.mp4 -->"), None);
        assert_eq!(video_of("<!-- a note -->"), None);
    }
}
