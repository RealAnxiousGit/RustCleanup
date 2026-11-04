//! The What's New video player in the Changelog window: the picture with
//! controls drawn over its bottom edge (play and pause, a seek bar with a
//! knob, sound, the time and full screen) and keyboard keys.
//!
//! Keys work while the pointer is over the video, after the video was
//! clicked, or in full screen: Space plays and pauses, F and Esc switch
//! full screen, M mutes, Left and Right seek 10 seconds, Up and Down change
//! the volume by 5%.

use eframe::egui::{self, Color32, CornerRadius, Key, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::theme::Palette;
use crate::i18n::{tr, trf};
use crate::sys::video::{self, Phase, Player, Status};

/// How long the controls stay after the pointer stops while playing.
const IDLE: f64 = 2.5;
const SEEK_STEP: f64 = 10.0;
const VOLUME_STEP: f32 = 0.05;
const WHITE: Color32 = Color32::WHITE;

#[derive(Default)]
pub struct View {
    player: Option<Player>,
    /// The URL `player` plays.
    url: String,
    picture: Option<egui::TextureHandle>,
    /// Wanted full screen state of the window.
    fullscreen: bool,
    /// The window reported full screen since it was asked for, so a later
    /// "not full screen" means the system left it.
    fullscreen_seen: bool,
    /// Last time the pointer moved over the video or a key was used.
    active_at: f64,
    /// Where the pointer was over a video last frame, to notice it moving.
    last_pointer: Option<Pos2>,
    /// The video was clicked last, so keys go to it.
    focused: bool,
    /// The seek bar is being dragged, at this fraction.
    scrub: Option<f32>,
    /// A short note over the picture, such as the volume, until a time.
    toast: Option<(String, f64)>,
    /// The volume slider over the speaker button is being dragged, at this level.
    volume_drag: Option<f32>,
    /// The volume slider was open last frame.
    volume_open: bool,
    /// The volume slider stays open until this time after a key or the wheel.
    volume_until: f64,
    /// Wheel movement over the speaker not yet turned into volume steps.
    wheel: f32,
    /// Where each video was drawn last frame, for the keys.
    zones: Vec<(String, Rect)>,
    /// The window's outer rectangle, inner size and maximized state from
    /// before full screen, to go back to.
    #[cfg(windows)]
    restore: Option<(Rect, Vec2, bool)>,
    /// The full screen window is kept above the taskbar (only while it has
    /// the focus, so switching to another program shows that program).
    #[cfg(windows)]
    on_top: bool,
}

impl View {
    /// The video at `url` could not be played.
    pub fn failed(&self, url: &str) -> bool {
        self.url == url && matches!(self.status().map(|s| s.phase), Some(Phase::Failed(_)))
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Stops the sound and frees the decoder, with the window.
    pub fn stop(&mut self, ctx: &egui::Context) {
        self.set_fullscreen(ctx, false);
        *self = Self::default();
    }

    fn start(&mut self, ctx: &egui::Context, url: &str) {
        let ctx2 = ctx.clone();
        let viewport = ctx.viewport_id();
        self.picture = None;
        self.url = url.to_string();
        self.player = Some(Player::start(url, move || ctx2.request_repaint_of(viewport)));
    }

    fn status(&self) -> Option<Status> {
        self.player.as_ref().map(Player::status)
    }

    fn ready(&self) -> Option<(&Player, Status)> {
        let player = self.player.as_ref()?;
        let status = player.status();
        let ok = matches!(status.phase, Phase::Playing | Phase::Paused | Phase::Ended);
        ok.then_some((player, status))
    }

    /// Plays, pauses or starts over; starts the video at `url` if it is not
    /// the loaded one.
    fn toggle(&mut self, ctx: &egui::Context, url: &str) {
        let phase = self.status().filter(|_| self.url == url).map(|s| s.phase);
        match phase {
            None | Some(Phase::Failed(_)) => self.start(ctx, url),
            Some(Phase::Playing) => self.player.as_ref().unwrap().pause(),
            Some(Phase::Downloading(_) | Phase::Loading) => {}
            Some(_) => self.player.as_ref().unwrap().play(),
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        if self.fullscreen != on {
            self.fullscreen = on;
            self.fullscreen_seen = false;
            #[cfg(windows)]
            self.cover_monitor(ctx, on);
            #[cfg(not(windows))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        }
    }

    /// Full screen on Windows: a window without a frame that covers its
    /// monitor, kept above the taskbar. With OpenGL, real full screen
    /// stopped showing new frames (the picture froze at the old window size).
    #[cfg(windows)]
    fn cover_monitor(&mut self, ctx: &egui::Context, on: bool) {
        use egui::ViewportCommand as Cmd;
        let info = ctx.input(|i| i.viewport().clone());
        if on {
            let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) else {
                ctx.send_viewport_cmd(Cmd::Fullscreen(true));
                return;
            };
            let scale = info.native_pixels_per_point.unwrap_or(1.0);
            let Some(monitor) = crate::sys::monitor_rect(outer.center() * scale) else {
                ctx.send_viewport_cmd(Cmd::Fullscreen(true));
                return;
            };
            let maximized = info.maximized.unwrap_or(false);
            self.restore = Some((outer, inner.size(), maximized));
            if maximized {
                ctx.send_viewport_cmd(Cmd::Maximized(false));
            }
            ctx.send_viewport_cmd(Cmd::Decorations(false));
            ctx.send_viewport_cmd(Cmd::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            self.on_top = true;
            ctx.send_viewport_cmd(Cmd::OuterPosition(monitor.min / scale));
            ctx.send_viewport_cmd(Cmd::InnerSize(monitor.size() / scale));
        } else if let Some((outer, inner, maximized)) = self.restore.take() {
            ctx.send_viewport_cmd(Cmd::WindowLevel(egui::WindowLevel::Normal));
            self.on_top = false;
            ctx.send_viewport_cmd(Cmd::Decorations(true));
            ctx.send_viewport_cmd(Cmd::InnerSize(inner));
            ctx.send_viewport_cmd(Cmd::OuterPosition(outer.min));
            if maximized {
                ctx.send_viewport_cmd(Cmd::Maximized(true));
            }
        } else {
            ctx.send_viewport_cmd(Cmd::Fullscreen(false));
        }
    }

    fn toggle_mute(&self) {
        if let Some((player, status)) = self.ready() {
            player.set_muted(!status.muted);
        }
    }

    fn seek_by(&self, seconds: f64) {
        if let Some((player, status)) = self.ready() {
            player.seek((status.position + seconds).clamp(0.0, status.duration.max(0.0)));
        }
    }

    fn volume_by(&mut self, step: f32, now: f64) {
        let Some((player, status)) = self.ready() else {
            return;
        };
        let mut volume = if status.muted && step > 0.0 { 0.0 } else { status.volume };
        // Whole steps, so 5% at a time stays on round numbers.
        volume = ((volume + step) / VOLUME_STEP).round() * VOLUME_STEP;
        let volume = volume.clamp(0.0, 1.0);
        if status.muted && step > 0.0 {
            player.set_muted(false);
        }
        player.set_volume(volume);
        let percent = (volume * 100.0).round() as u32;
        self.toast = Some((trf("Volume {0}%", &[&percent]), now + 1.5));
        self.volume_until = now + 1.5;
        self.active_at = now;
    }

    /// Takes the player's keys before the window's own widgets see them.
    /// Call at the start of the window's frame. Returns true when Esc was
    /// used to leave full screen, so the window does not close.
    pub fn keys(&mut self, ctx: &egui::Context) -> bool {
        let now = ctx.input(|i| i.time);
        // Follow the system leaving full screen (such as Win+Down).
        let reported = ctx.input(|i| i.viewport().fullscreen);
        if self.fullscreen {
            match reported {
                Some(true) => self.fullscreen_seen = true,
                Some(false) if self.fullscreen_seen => {
                    self.fullscreen = false;
                    self.fullscreen_seen = false;
                }
                _ => {}
            }
        }
        #[cfg(windows)]
        if self.restore.is_some() {
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
            if focused != self.on_top {
                self.on_top = focused;
                let level = if focused { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            }
        }
        // A click elsewhere in the window takes the keys away again.
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let over = |zones: &[(String, Rect)]| {
            pointer.and_then(|p| zones.iter().find(|(_, r)| r.contains(p)).map(|(u, _)| u.clone()))
        };
        let hovered = over(&self.zones);
        if ctx.input(|i| i.pointer.any_pressed()) && hovered.is_none() && !self.fullscreen {
            self.focused = false;
        }
        let target = if self.fullscreen {
            Some(self.url.clone())
        } else {
            hovered.or_else(|| self.focused.then(|| self.url.clone()))
        };
        self.zones.clear();
        let Some(target) = target else {
            return false;
        };
        let loaded = target == self.url && self.player.is_some();
        let press = |key: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));

        let mut used = false;
        let mut left_fullscreen = false;
        if press(Key::Space) {
            self.toggle(ctx, &target);
            used = true;
        }
        if self.fullscreen && press(Key::Escape) {
            self.set_fullscreen(ctx, false);
            left_fullscreen = true;
            used = true;
        }
        if loaded {
            if press(Key::F) {
                let on = !self.fullscreen;
                self.set_fullscreen(ctx, on);
                used = true;
            }
            if press(Key::M) {
                self.toggle_mute();
                used = true;
            }
            if press(Key::ArrowRight) {
                self.seek_by(SEEK_STEP);
                used = true;
            }
            if press(Key::ArrowLeft) {
                self.seek_by(-SEEK_STEP);
                used = true;
            }
            if press(Key::ArrowUp) {
                self.volume_by(VOLUME_STEP, now);
                used = true;
            }
            if press(Key::ArrowDown) {
                self.volume_by(-VOLUME_STEP, now);
                used = true;
            }
        }
        if used {
            self.focused = true;
            self.active_at = now;
            ctx.request_repaint();
        }
        left_fullscreen
    }

    /// The video as a 16:9 card in the notes.
    pub fn card(&mut self, ui: &mut Ui, pal: &Palette, url: &str, title: &str) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::hover());
        self.draw(ui, pal, rect, url, title, 8);
    }

    /// The loaded video filling the window, for full screen.
    pub fn full(&mut self, ui: &mut Ui, pal: &Palette) {
        let rect = ui.max_rect();
        let url = self.url.clone();
        self.draw(ui, pal, rect, &url, "", 0);
    }

    fn draw(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect, url: &str, title: &str, round: u8) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let radius = CornerRadius::same(round);
        let id = egui::Id::unique(("whats_new_video", url));
        let visible = ui.clip_rect().intersect(rect);
        if visible.is_positive() {
            self.zones.push((url.to_string(), visible));
        }
        let mine = self.url == url && self.player.is_some();

        // Newest picture to the GPU.
        if mine && let Some(frame) = self.player.as_ref().and_then(Player::take_frame) {
            let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
            match &mut self.picture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.picture =
                        Some(ctx.load_texture("whats_new_video", image, egui::TextureOptions::LINEAR))
                }
            }
        }
        let status = self.status().filter(|_| mine);
        let phase = status.as_ref().map(|s| s.phase.clone());

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, radius, Color32::from_rgb(0x10, 0x10, 0x12));
        if mine && let Some(texture) = &self.picture {
            // Letterboxed to keep the video's shape.
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let shown = Rect::from_center_size(rect.center(), size * scale);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture.id(), shown, uv, WHITE);
        }

        // The picture: a click plays and pauses, a double click is full screen.
        let response = ui.interact(rect, id, Sense::click());
        let ready = matches!(phase, Some(Phase::Playing | Phase::Paused | Phase::Ended));
        // The pointer coming in or moving over the video shows the controls.
        if let Some(at) = response.hover_pos() {
            if self.last_pointer != Some(at) {
                self.active_at = now;
            }
            self.last_pointer = Some(at);
        }
        let playing = matches!(phase, Some(Phase::Playing));
        let shown_controls = ready
            && (!playing
                || self.scrub.is_some()
                || self.volume_drag.is_some()
                || self.volume_open
                || now - self.active_at < IDLE
                || (response.hovered() && !self.fullscreen && near_bottom(ui, rect)));
        if response.clicked() {
            self.focused = true;
            self.active_at = now;
            self.toggle(&ctx, url);
        }
        if response.double_clicked() && ready {
            let on = !self.fullscreen;
            self.set_fullscreen(&ctx, on);
        }
        let name = if playing { tr("Pause") } else { tr("Play") };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));

        // The middle of the picture: a round play button before it plays,
        // or what the player is doing.
        let center = rect.center();
        let text = |painter: &egui::Painter, at: Pos2, s: &str, color: Color32| {
            painter.text(at, egui::Align2::CENTER_CENTER, s, egui::FontId::proportional(14.0), color);
        };
        let big_play = |label: Option<&str>| {
            let fill = if response.hovered() { pal.accent_hover } else { pal.accent };
            painter.circle_filled(center, 30.0, fill);
            play_icon(&painter, center + vec2(2.0, 0.0), 13.0, pal.on_accent);
            if let Some(label) = label {
                text(&painter, center + vec2(0.0, 50.0), label, WHITE);
            }
        };
        match &phase {
            None => big_play(Some(if title.is_empty() { tr("Watch what's new") } else { title })),
            Some(Phase::Downloading(p)) => {
                text(&painter, center, &trf("Downloading the video, {0}%", &[p]), WHITE)
            }
            Some(Phase::Loading) => text(&painter, center, tr("Opening the video."), WHITE),
            Some(Phase::Ended) => big_play(Some(tr("Watch again"))),
            Some(Phase::Failed(e)) => text(&painter, center, e, pal.error),
            Some(Phase::Paused) if self.scrub.is_none() => big_play(None),
            _ => {}
        }

        if let (Some(status), true) = (&status, shown_controls) {
            self.controls(ui, pal, rect, round, status, id, now);
        } else if playing && self.fullscreen {
            // The pointer hides with the controls in full screen.
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        if playing && shown_controls && self.scrub.is_none() {
            // Wakes up to hide the controls once the pointer rests.
            let left = IDLE - (now - self.active_at);
            if left > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left + 0.05));
            }
        }

        if let Some((note, until)) = &self.toast {
            if now < *until && mine {
                let galley = painter.layout_no_wrap(note.clone(), egui::FontId::proportional(15.0), WHITE);
                // At the top of the part of the video in view.
                let at = pos2(center.x, visible.top().max(rect.top()) + 28.0);
                let bg = Rect::from_center_size(at, galley.size() + vec2(20.0, 10.0));
                painter.rect_filled(bg, CornerRadius::same(6), Color32::from_black_alpha(170));
                painter.galley(bg.min + vec2(10.0, 5.0), galley, WHITE);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(*until - now + 0.05));
            } else {
                self.toast = None;
            }
        }
        if round > 0 {
            painter.rect_stroke(rect, radius, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn controls(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        rect: Rect,
        round: u8,
        status: &Status,
        id: egui::Id,
        now: f64,
    ) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        let pad = 12.0;
        let band = 78.0_f32.min(rect.height() * 0.5);

        // A dark fade up from the bottom edge, so white icons read on any frame.
        let mut mesh = egui::Mesh::default();
        let top = rect.bottom() - band;
        let clear = Color32::from_black_alpha(0);
        let dark = Color32::from_black_alpha(175);
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(rect.left(), top), clear);
        mesh.colored_vertex(pos2(rect.right(), top), clear);
        mesh.colored_vertex(pos2(rect.right(), rect.bottom() - f32::from(round)), dark);
        mesh.colored_vertex(pos2(rect.left(), rect.bottom() - f32::from(round)), dark);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
        painter.add(egui::Shape::mesh(mesh));
        if round > 0 {
            // The rounded bottom corners, under the fade.
            let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - f32::from(round)), rect.max);
            painter.rect_filled(
                foot,
                CornerRadius { nw: 0, ne: 0, sw: round, se: round },
                dark,
            );
        }

        // The seek bar: a track, the played part, and a knob at the position.
        let duration = status.duration.max(0.0);
        let track = Rect::from_min_max(
            pos2(rect.left() + pad, rect.bottom() - 46.0),
            pos2(rect.right() - pad, rect.bottom() - 46.0),
        );
        let hit = Rect::from_center_size(track.center(), vec2(track.width() + 12.0, 18.0));
        let seek = ui.interact(hit, id.with("seek"), Sense::click_and_drag());
        let at_pointer = |r: &egui::Response| {
            r.interact_pointer_pos()
                .or_else(|| r.hover_pos())
                .map(|p| ((p.x - track.left()) / track.width()).clamp(0.0, 1.0))
        };
        if seek.is_pointer_button_down_on() || seek.dragged() {
            self.scrub = at_pointer(&seek);
            self.active_at = now;
        } else if let Some(f) = self.scrub.take()
            && let Some(player) = &self.player
        {
            // Seeks once, on release.
            player.seek(f64::from(f) * duration);
        }
        let fraction = self.scrub.unwrap_or(if duration > 0.0 {
            (status.position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        });
        let lit = seek.hovered() || self.scrub.is_some();
        let thick = if lit { 5.0 } else { 3.5 };
        let line = |from: f32, to: f32, color: Color32| {
            let r = Rect::from_min_max(
                pos2(track.left() + track.width() * from, track.center().y - thick / 2.0),
                pos2(track.left() + track.width() * to, track.center().y + thick / 2.0),
            );
            painter.rect_filled(r, CornerRadius::same(3), color);
        };
        line(0.0, 1.0, Color32::from_white_alpha(70));
        line(0.0, fraction, pal.accent);
        let knob = pos2(track.left() + track.width() * fraction, track.center().y);
        painter.circle_filled(knob, if lit { 7.5 } else { 6.0 }, pal.accent);
        painter.circle_stroke(knob, if lit { 7.5 } else { 6.0 }, Stroke::new(1.5, WHITE));
        if let Some(f) = at_pointer(&seek).filter(|_| seek.hovered() || self.scrub.is_some()) {
            // The time under the pointer, above the bar.
            let x = track.left() + track.width() * self.scrub.unwrap_or(f);
            let label = video::clock(f64::from(self.scrub.unwrap_or(f)) * duration);
            let galley = painter.layout_no_wrap(label, egui::FontId::proportional(13.0), WHITE);
            let size = galley.size() + vec2(12.0, 6.0);
            let left = (x - size.x / 2.0).clamp(rect.left() + 4.0, rect.right() - size.x - 4.0);
            let bg = Rect::from_min_size(pos2(left, track.top() - 14.0 - size.y), size);
            painter.rect_filled(bg, CornerRadius::same(4), Color32::from_black_alpha(200));
            painter.galley(bg.min + vec2(6.0, 3.0), galley, WHITE);
        }
        seek.on_hover_cursor(egui::CursorIcon::PointingHand)
            .widget_info(|| egui::WidgetInfo::slider(true, f64::from(fraction) * duration, ""));

        // The buttons under the bar.
        let row = rect.bottom() - 20.0;
        let size = 30.0;
        let button = |ui: &mut Ui, x: f32, key: &str, name: &str| {
            let r = Rect::from_center_size(pos2(x, row), Vec2::splat(size));
            let response = ui
                .interact(r, id.with(key), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.hovered() {
                painter.circle_filled(r.center(), size / 2.0, Color32::from_white_alpha(38));
            }
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            (r.center(), response)
        };

        let playing = matches!(status.phase, Phase::Playing);
        let x = rect.left() + pad + size / 2.0;
        let (name, keyname) = if playing { (tr("Pause"), "Space") } else { (tr("Play"), "Space") };
        let (c, play) = button(ui, x, "play", name);
        if playing {
            pause_icon(&painter, c, 7.0, WHITE);
        } else {
            play_icon(&painter, c + vec2(1.5, 0.0), 7.5, WHITE);
        }
        if play.on_hover_text(format!("{name} ({keyname})")).clicked() {
            self.focused = true;
            let url = self.url.clone();
            self.toggle(&ctx, &url);
        }

        let x = x + size + 4.0;
        let silent = status.muted || status.volume <= 0.0;
        let name = if silent { tr("Unmute") } else { tr("Mute") };
        let (c, sound) = button(ui, x, "sound", name);
        speaker_icon(&painter, c, if silent { None } else { Some(status.volume) }, WHITE);
        self.volume_slider(ui, pal, &painter, rect, c, size, status, &sound, id, now);
        if sound.clicked() {
            self.focused = true;
            if let Some(player) = &self.player {
                if status.volume <= 0.0 {
                    player.set_muted(false);
                    player.set_volume(0.5);
                } else {
                    player.set_muted(!status.muted);
                }
            }
        }

        let time = format!("{} / {}", video::clock(status.position), video::clock(duration));
        painter.text(
            pos2(x + size / 2.0 + 8.0, row),
            egui::Align2::LEFT_CENTER,
            time,
            egui::FontId::proportional(13.5),
            WHITE,
        );

        let x = rect.right() - pad - size / 2.0;
        let name = if self.fullscreen { tr("Exit full screen") } else { tr("Full screen") };
        let (c, full) = button(ui, x, "full", name);
        fullscreen_icon(&painter, c, 8.0, self.fullscreen, WHITE);
        if full.on_hover_text(format!("{name} (F)")).clicked() {
            self.focused = true;
            let on = !self.fullscreen;
            self.set_fullscreen(&ctx, on);
        }
    }
}

impl View {
    /// The volume slider that opens above the speaker button: it shows while
    /// the pointer is on the button or the slider, while dragging, and for a
    /// moment after Up, Down or the wheel. Click or drag it to set the volume;
    /// the wheel over the button or the slider moves it 5% a notch.
    #[allow(clippy::too_many_arguments)]
    fn volume_slider(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        painter: &egui::Painter,
        rect: Rect,
        button: Pos2,
        size: f32,
        status: &Status,
        sound: &egui::Response,
        id: egui::Id,
        now: f64,
    ) {
        let width = 30.0;
        // Above the seek bar, so the two never overlap.
        let bottom = (rect.bottom() - 46.0 - 12.0).min(button.y - size / 2.0 - 6.0);
        let top = (bottom - 120.0).max(rect.top() + 4.0);
        let pop = Rect::from_min_max(pos2(button.x - width / 2.0, top), pos2(button.x + width / 2.0, bottom));
        // Covers the gap down to the button, so moving up to the slider keeps it open.
        let reach = pop.union(Rect::from_center_size(button, Vec2::splat(size)));
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = pointer.is_some_and(|p| reach.contains(p)) && ui.rect_contains_pointer(rect);
        let open = sound.hovered()
            || (self.volume_open && over)
            || self.volume_drag.is_some()
            || now < self.volume_until;
        self.volume_open = open;
        if now < self.volume_until {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(self.volume_until - now + 0.05));
        }

        // The wheel over the speaker or the open slider, instead of scrolling the window.
        if sound.hovered() || (open && over) {
            let delta = ui.input_mut(|i| {
                let mut lines = 0.0;
                for e in &i.events {
                    if let egui::Event::MouseWheel { unit, delta, .. } = e {
                        lines += match unit {
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Point => delta.y / 50.0,
                            egui::MouseWheelUnit::Page => delta.y * 3.0,
                        };
                    }
                }
                i.smooth_scroll_delta = Vec2::ZERO;
                lines
            });
            self.wheel += delta;
            while self.wheel >= 1.0 {
                self.wheel -= 1.0;
                self.volume_by(VOLUME_STEP, now);
            }
            while self.wheel <= -1.0 {
                self.wheel += 1.0;
                self.volume_by(-VOLUME_STEP, now);
            }
        } else {
            self.wheel = 0.0;
        }
        if !open {
            return;
        }

        let track = Rect::from_min_max(pos2(pop.center().x - 2.0, pop.top() + 30.0), pos2(pop.center().x + 2.0, pop.bottom() - 14.0));
        let response = ui.interact(pop, id.with("volume"), Sense::click_and_drag());
        let level_at = |y: f32| ((track.bottom() - y) / track.height()).clamp(0.0, 1.0);
        if response.is_pointer_button_down_on() || response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let v = (level_at(p.y) * 100.0).round() / 100.0;
                // Windows changes the volume as it goes. ffplay on Linux has to
                // restart for each change, so there it is set on release.
                if cfg!(windows) && self.volume_drag != Some(v) {
                    self.apply_volume(v);
                }
                self.volume_drag = Some(v);
            }
            self.focused = true;
            self.active_at = now;
        } else if let Some(v) = self.volume_drag.take() {
            self.apply_volume(v);
        }
        let level = self.volume_drag.unwrap_or(if status.muted { 0.0 } else { status.volume });
        let lit = response.hovered() || self.volume_drag.is_some();

        // A grey pill with a thin edge, so it reads over dark frames too.
        painter.rect_filled(pop, CornerRadius::same(15), Color32::from_rgba_unmultiplied(38, 38, 42, 235));
        painter.rect_stroke(pop, CornerRadius::same(15), Stroke::new(1.0, Color32::from_white_alpha(45)), egui::StrokeKind::Inside);
        let percent = format!("{}", (level * 100.0).round() as u32);
        painter.text(
            pos2(pop.center().x, pop.top() + 14.0),
            egui::Align2::CENTER_CENTER,
            percent,
            egui::FontId::proportional(12.0),
            WHITE,
        );
        painter.rect_filled(track, CornerRadius::same(2), Color32::from_white_alpha(70));
        let y = track.bottom() - track.height() * level;
        let fill = Rect::from_min_max(pos2(track.left(), y), track.max);
        painter.rect_filled(fill, CornerRadius::same(2), pal.accent);
        let r = if lit { 7.5 } else { 6.0 };
        painter.circle_filled(pos2(track.center().x, y), r, pal.accent);
        painter.circle_stroke(pos2(track.center().x, y), r, Stroke::new(1.5, WHITE));
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .widget_info(|| egui::WidgetInfo::slider(true, f64::from(level) * 100.0, tr("Volume")));
    }

    /// Sets the volume from the slider; above zero also unmutes.
    fn apply_volume(&self, volume: f32) {
        if let Some((player, status)) = self.ready() {
            if status.muted && volume > 0.0 {
                player.set_muted(false);
            }
            player.set_volume(volume);
        }
    }
}

/// The pointer is in the bottom part of the video, where the controls are.
fn near_bottom(ui: &Ui, rect: Rect) -> bool {
    ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| p.y > rect.bottom() - 70.0)
}

fn play_icon(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    let points = vec![c + vec2(-r * 0.75, -r), c + vec2(-r * 0.75, r), c + vec2(r, 0.0)];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn pause_icon(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    let bar = vec2(r * 0.55, r * 2.0);
    for dx in [-r * 0.5, r * 0.5] {
        painter.rect_filled(Rect::from_center_size(c + vec2(dx, 0.0), bar), CornerRadius::same(1), color);
    }
}

/// A speaker with sound waves for the volume, or a cross when silent.
fn speaker_icon(painter: &egui::Painter, c: Pos2, volume: Option<f32>, color: Color32) {
    let o = c + vec2(-4.0, 0.0);
    painter.rect_filled(
        Rect::from_min_max(o + vec2(-4.5, -3.0), o + vec2(-1.0, 3.0)),
        CornerRadius::same(1),
        color,
    );
    let cone = vec![o + vec2(-1.5, -3.0), o + vec2(3.5, -7.0), o + vec2(3.5, 7.0), o + vec2(-1.5, 3.0)];
    painter.add(egui::Shape::convex_polygon(cone, color, Stroke::NONE));
    let stroke = Stroke::new(1.6, color);
    match volume {
        None => {
            let x = o + vec2(9.5, 0.0);
            let d = 3.2;
            painter.line_segment([x + vec2(-d, -d), x + vec2(d, d)], stroke);
            painter.line_segment([x + vec2(-d, d), x + vec2(d, -d)], stroke);
        }
        Some(v) => {
            let waves = if v < 0.5 { 1 } else { 2 };
            for w in 0..waves {
                let radius = 5.5 + w as f32 * 4.0;
                let points = (0..=12)
                    .map(|k| {
                        let a = -0.9 + 1.8 * k as f32 / 12.0;
                        o + vec2(3.5, 0.0) + radius * vec2(a.cos(), a.sin())
                    })
                    .collect();
                painter.add(egui::Shape::line(points, stroke));
            }
        }
    }
}

/// Four corners pointing out (enter full screen) or in (leave it).
fn fullscreen_icon(painter: &egui::Painter, c: Pos2, r: f32, inward: bool, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let arm = r * 0.75;
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let corner = if inward {
            c + vec2(sx * r * 0.3, sy * r * 0.3)
        } else {
            c + vec2(sx * r, sy * r)
        };
        // The arms point away from the middle when leaving full screen.
        let dir = if inward { 1.0 } else { -1.0 };
        let points = vec![
            corner + vec2(sx * arm * dir, 0.0),
            corner,
            corner + vec2(0.0, sy * arm * dir),
        ];
        painter.add(egui::Shape::line(points, stroke));
    }
}
