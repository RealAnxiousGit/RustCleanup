//! The Changelog window: the GitHub release notes of every version, newest
//! first, in one scrolling page, with a side menu that jumps to any version.
//! It opens by itself on the first start after an update. (The update window
//! shows only the new version's notes; this window is the full history.)

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke, Ui};

use super::theme::{self, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{tr, trf};
use crate::sys::updater::{self, ReleaseNotes};
use crate::sys::video::{self, Phase, Player};

pub const WINDOW: &str = "changelog_window";
const RELEASES_PAGE: &str = "https://github.com/RealAnxiousGit/RustCleanup/releases";

#[derive(Default)]
struct State {
    open: bool,
    /// Every release, newest first, or why they could not be loaded.
    releases: Option<Result<Vec<ReleaseNotes>, String>>,
    /// The side menu of versions is showing.
    menu: bool,
    /// The version (index into `releases`) to scroll to on the next frame.
    jump: Option<usize>,
    /// The What's New video, once its play button was pressed, and its URL.
    player: Option<Player>,
    player_url: String,
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

/// The notes without a first heading that only repeats the release's title,
/// which older releases have.
fn without_title(release: &ReleaseNotes) -> &str {
    let text = release.markdown.trim_start();
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let heading = first.trim_start_matches('#').trim();
    let version = heading.trim_start_matches("RustCleanup ").trim_start_matches('v');
    let repeats =
        first.starts_with('#') && (heading == release.title || version == release.version);
    if repeats { rest } else { text }
}

/// Where to open: the running version, unless it is the newest (the top).
fn start_at(all: &[ReleaseNotes], version: &str) -> Option<usize> {
    all.iter().position(|r| r.version == version).filter(|&i| i > 0)
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
        if !matches!(state.releases, Some(Ok(_))) {
            state.releases = None;
            drop(state);
            self.fetch(ctx);
        }
    }

    /// Downloads the notes on a worker thread.
    fn fetch(&self, ctx: &egui::Context) {
        let state = self.state.clone();
        let ctx = ctx.clone();
        self.jobs.start(Job::Changelog, move |_| {
            let result = updater::all_notes();
            let mut s = lock(&state);
            s.jump = result
                .as_ref()
                .ok()
                .and_then(|all| start_at(all, env!("CARGO_PKG_VERSION")));
            s.releases = Some(result);
            drop(s);
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
            super::window_builder(tr("Changelog"), [640.0, 660.0], [460.0, 380.0]),
            move |ui, _class| {
                let pal = theme::palette();
                let mut close = super::close_pressed(ui.ctx());
                let mut retry = false;
                let releases = lock(&state).releases.clone();
                let loaded = releases.as_ref().and_then(|r| r.as_ref().ok());
                let current = env!("CARGO_PKG_VERSION");

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
                            ui.add_space(6.0);
                            let page = loaded
                                .and_then(|all| all.iter().find(|r| r.version == current))
                                .map(|r| r.page.as_str())
                                .filter(|p| !p.is_empty())
                                .unwrap_or(RELEASES_PAGE);
                            widgets::link(ui, &jobs.log, tr("Open the release page"), page);
                        });
                    });

                egui::Panel::top("changelog_heading")
                    .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                        left: 22,
                        right: 22,
                        top: 18,
                        bottom: 10,
                    }))
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let menu = lock(&state).menu;
                            let button =
                                widgets::menu_button(ui, &pal, menu, tr("Jump to a version"));
                            if button.clicked() && loaded.is_some() {
                                lock(&state).menu = !menu;
                            }
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(trf("What's new in RustCleanup {0}", &[&current]))
                                    .family(semibold())
                                    .size(20.0),
                            );
                        });
                        ui.add_space(6.0);
                        widgets::note(
                            ui,
                            &pal,
                            tr("Every version's notes, from RustCleanup's releases on GitHub."),
                        );
                    });

                if let Some(all) = loaded {
                    let mut menu = lock(&state).menu;
                    egui::Panel::left("changelog_versions")
                        .resizable(false)
                        .exact_size(210.0)
                        .frame(
                            egui::Frame::new()
                                .fill(pal.surface)
                                .inner_margin(Margin::symmetric(10, 12)),
                        )
                        .show_collapsible(ui, &mut menu, |ui| {
                            ui.label(RichText::new(tr("Versions")).family(semibold()).size(15.0));
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical()
                                .id_salt("changelog_version_list")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for (i, release) in all.iter().enumerate() {
                                        let row =
                                            version_row(ui, &pal, release, release.version == current);
                                        if row.clicked() {
                                            lock(&state).jump = Some(i);
                                        }
                                    }
                                });
                        });
                }

                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                        left: 22,
                        right: 12,
                        top: 8,
                        bottom: 8,
                    }))
                    .show(ui, |ui| match &releases {
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
                        Some(Ok(all)) => {
                            let jump = lock(&state).jump.take();
                            egui::ScrollArea::vertical()
                                .id_salt("changelog_notes")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    // Each video shows once, with the newest release linking it.
                                    let mut videos: Vec<&str> = Vec::new();
                                    for (i, release) in all.iter().enumerate() {
                                        if i > 0 {
                                            ui.add_space(14.0);
                                            ui.separator();
                                            ui.add_space(10.0);
                                        }
                                        let top = ui.cursor().min.y;
                                        release_heading(ui, &pal, release, release.version == current);
                                        if jump == Some(i) {
                                            let rect = egui::Rect::from_min_max(
                                                egui::pos2(ui.min_rect().left(), top - 6.0),
                                                egui::pos2(ui.min_rect().right(), top + 40.0),
                                            );
                                            ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                                        }
                                        ui.add_space(8.0);
                                        if let Some((url, title)) = video_of(&release.markdown)
                                            && !videos.contains(&url)
                                        {
                                            videos.push(url);
                                            video_card(ui, &pal, &state, &jobs, url, title);
                                            ui.add_space(14.0);
                                        }
                                        super::update::notes(ui, &pal, without_title(release));
                                    }
                                    ui.add_space(12.0);
                                });
                        }
                    });

                if retry {
                    lock(&state).releases = None;
                    refetch.fetch(ui.ctx());
                }
                if close {
                    let mut s = lock(&state);
                    s.open = false;
                    s.menu = false;
                    // Stops the sound and frees the decoder with the window.
                    s.player = None;
                    s.player_url.clear();
                    s.picture = None;
                    drop(s);
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

/// A version's title, with its release date under it.
fn release_heading(ui: &mut Ui, pal: &theme::Palette, release: &ReleaseNotes, current: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&release.title).family(semibold()).size(19.0));
        if current {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(pal.accent)
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(RichText::new(tr("This version")).color(pal.on_accent).size(12.0));
                });
        }
    });
    if let Some(date) = &release.published {
        ui.label(RichText::new(date).color(pal.muted).size(14.0));
    }
}

/// One entry in the side menu: the version's title and date. Clicking it
/// scrolls the notes to that version.
fn version_row(
    ui: &mut Ui,
    pal: &theme::Palette,
    release: &ReleaseNotes,
    current: bool,
) -> egui::Response {
    let width = ui.available_width();
    let galley = |text: &str, size: f32, bold: bool| {
        let mut rich = RichText::new(text).size(size);
        if bold {
            rich = rich.family(semibold());
        }
        egui::WidgetText::from(rich).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            width - 18.0,
            egui::TextStyle::Body,
        )
    };
    let title = galley(&release.title, 14.0, true);
    let date = galley(release.published.as_deref().unwrap_or_default(), 12.0, false);
    let height = 8.0 + title.size().y + 2.0 + date.size().y + 8.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &release.title));
    if ui.is_rect_visible(rect) {
        let hover = widgets::fade(ui, response.id, response.hovered());
        let lit = ui.visuals().widgets.hovered.weak_bg_fill;
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), widgets::blend(pal.surface, lit, hover));
        if current {
            // A thin accent bar marks the running version.
            let bar = egui::Rect::from_min_size(
                rect.min + egui::vec2(0.0, 7.0),
                egui::vec2(3.0, height - 14.0),
            );
            ui.painter().rect_filled(bar, CornerRadius::same(2), pal.accent);
        }
        let title_pos = egui::pos2(rect.left() + 10.0, rect.top() + 8.0);
        let date_pos = egui::pos2(title_pos.x, title_pos.y + title.size().y + 2.0);
        ui.painter().galley(title_pos, title, pal.text);
        ui.painter().galley(date_pos, date, pal.muted);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    // Only the card whose video is loaded shows the player.
    let mine = s.player_url == url;
    // Newest picture to the GPU.
    if let Some(frame) = s.player.as_ref().filter(|_| mine).and_then(Player::take_frame) {
        let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
        match &mut s.picture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                s.picture =
                    Some(ui.ctx().load_texture("whats_new_video", image, egui::TextureOptions::LINEAR))
            }
        }
    }
    let status = s.player.as_ref().filter(|_| mine).map(Player::status);
    if let Some(texture) = s.picture.as_ref().filter(|_| mine) {
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
    let toggle = |s: &mut State, ui: &Ui| match s.player.as_ref().filter(|_| mine).map(|p| p.status().phase) {
        None | Some(Phase::Failed(_)) => {
            let ctx = ui.ctx().clone();
            let viewport = super::window_id(WINDOW);
            s.picture = None;
            s.player_url = url.to_string();
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

    fn release(version: &str, title: &str, markdown: &str) -> ReleaseNotes {
        ReleaseNotes {
            version: version.into(),
            title: title.into(),
            markdown: markdown.into(),
            page: String::new(),
            published: None,
        }
    }

    #[test]
    fn drops_a_heading_that_repeats_the_title() {
        let r = release("2.0.1", "RustCleanup 2.0.1", "# RustCleanup 2.0.1\nPortable.");
        assert_eq!(without_title(&r), "Portable.");
        let r = release("2.0.0", "RustCleanup v2.0.0", "## RustCleanup v2.0.0\nText");
        assert_eq!(without_title(&r), "Text");
        let r = release("3.0.1", "RustCleanup 3.0.1", "## Added\n- x");
        assert_eq!(without_title(&r), "## Added\n- x");
    }

    #[test]
    fn opens_at_the_running_version() {
        let all = [release("3.0.2", "", ""), release("3.0.1", "", "")];
        assert_eq!(start_at(&all, "3.0.2"), None);
        assert_eq!(start_at(&all, "3.0.1"), Some(1));
        assert_eq!(start_at(&all, "9.9.9"), None);
    }

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
