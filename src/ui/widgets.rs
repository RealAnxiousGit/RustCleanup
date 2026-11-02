//! Small custom widgets shared by every panel.

use eframe::egui::{
    self, Color32, CornerRadius, Response, RichText, Role, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo,
};

use super::theme::{Palette, semibold};
use crate::events::Logger;
use crate::i18n::tr;
use crate::sys::shell;

/// Section heading in the semibold face.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).family(semibold()).size(16.0));
}

/// Secondary explanatory text.
pub fn note(ui: &mut Ui, pal: &Palette, text: impl Into<String>) -> Response {
    ui.label(RichText::new(text.into()).size(12.5).color(pal.muted))
}

/// An on/off switch. `label` is its accessible name for screen readers.
pub fn switch(ui: &mut Ui, pal: &Palette, on: bool, enabled: bool, label: &str) -> Response {
    let size = egui::vec2(40.0, 22.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| WidgetInfo::selected(Role::Switch, enabled, on, label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, on);
        let radius = rect.height() / 2.0;
        let mut fill = blend(pal.meter_track, pal.accent, t);
        if !enabled {
            fill = fill.gamma_multiply(0.55);
        }
        let stroke = if response.has_focus() {
            ui.visuals().selection.stroke
        } else if response.hovered() && enabled {
            Stroke::new(1.0, pal.muted)
        } else {
            Stroke::NONE
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(radius as u8),
            fill,
            stroke,
            StrokeKind::Outside,
        );
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let knob = if on || t > 0.5 {
            pal.on_accent
        } else {
            pal.surface
        };
        ui.painter().circle(
            egui::pos2(x, rect.center().y),
            radius - 3.0,
            knob,
            Stroke::new(1.0, pal.border),
        );
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A settings row: title and explanation on the left, switch on the right.
/// Returns the requested new value when the switch is clicked.
pub fn switch_row(
    ui: &mut Ui,
    pal: &Palette,
    title: &str,
    detail: &str,
    on: bool,
    enabled: bool,
) -> Option<bool> {
    let mut result = None;
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() - 64.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.label(RichText::new(title).family(semibold()));
            if !detail.is_empty() {
                note(ui, pal, detail);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if switch(ui, pal, on, enabled, title).clicked() {
                result = Some(!on);
            }
        });
    });
    result
}

/// Mixes two colours, alpha included; `t = 0` is `a`, `t = 1` is `b`.
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

/// A big figure, its description, and an optional percentage on the right.
pub fn figure_row(ui: &mut Ui, pal: &Palette, big: &str, rest: &str, percent: Option<f32>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(big)
                .family(semibold())
                .size(22.0)
                .color(pal.text),
        );
        ui.label(RichText::new(rest).color(pal.muted));
        if let Some(p) = percent {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{:.0}%", p * 100.0))
                        .family(semibold())
                        .size(15.0),
                );
            });
        }
    });
}

/// How full a usage bar is: 0 normal, 1 high (yellow), 2 very high (red).
pub fn usage_level(fraction: f32, high: f32, very_high: f32) -> u8 {
    if fraction > very_high {
        2
    } else if fraction > high {
        1
    } else {
        0
    }
}

/// A thin usage bar. Its fill fades to yellow at `level` 1 and red at 2.
pub fn meter(ui: &mut Ui, pal: &Palette, fraction: f32, level: u8) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), Sense::hover());
    let t = ui
        .ctx()
        .animate_value_with_time(response.id.with("level"), level.min(2) as f32, 0.25);
    let color = if t <= 1.0 {
        blend(pal.meter_fill, pal.warn, t)
    } else {
        blend(pal.warn, pal.error, t - 1.0)
    };
    let radius = CornerRadius::same(3);
    ui.painter().rect_filled(rect, radius, pal.meter_track);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, radius, color);
}

/// A separator with breathing room above and below.
pub fn divider(ui: &mut Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

/// How long hover and selection fades take.
pub const FADE_TIME: f32 = 0.12;

/// 0 to 1, easing towards 1 while `on` and back to 0 after.
pub fn fade(ui: &Ui, id: egui::Id, on: bool) -> f32 {
    ui.ctx().animate_bool_with_time(id, on, FADE_TIME)
}

/// Colours of a painted button in its resting, hovered and pressed states.
struct ButtonLook {
    rest: Color32,
    hover: Color32,
    press: Color32,
    stroke: Stroke,
    hover_stroke: Stroke,
    text: Color32,
    font: egui::FontId,
    min: egui::Vec2,
}

/// A button whose fill fades between states instead of switching at once.
fn painted_button(ui: &mut Ui, text: &str, enabled: bool, look: ButtonLook) -> Response {
    let padding = ui.spacing().button_padding;
    // The galley keeps the colour it is laid out with, so dim it here.
    let ink = if enabled {
        look.text
    } else {
        look.text.gamma_multiply(0.5)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), look.font, ink);
    let size = (galley.size() + padding * 2.0).max(look.min);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, text));
    if ui.is_rect_visible(rect) {
        let hover = fade(ui, response.id, enabled && response.hovered());
        let press = fade(ui, response.id.with("press"), enabled && response.is_pointer_button_down_on());
        let mut fill = blend(blend(look.rest, look.hover, hover), look.press, press);
        let mut stroke = Stroke::new(
            look.stroke.width,
            blend(look.stroke.color, look.hover_stroke.color, hover),
        );
        if !enabled {
            fill = fill.gamma_multiply(0.55);
            stroke.color = stroke.color.gamma_multiply(0.55);
        }
        let radius = CornerRadius::same(5);
        ui.painter()
            .rect(rect, radius, fill, stroke, StrokeKind::Inside);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                radius,
                ui.visuals().selection.stroke,
                StrokeKind::Outside,
            );
        }
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, ink);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The accent-filled button. Used for exactly one action per screen.
pub fn primary_button(ui: &mut Ui, pal: &Palette, text: &str, enabled: bool) -> Response {
    let font = egui::FontId::new(egui::TextStyle::Button.resolve(ui.style()).size, semibold());
    painted_button(
        ui,
        text,
        enabled,
        ButtonLook {
            rest: pal.accent,
            hover: pal.accent_hover,
            press: blend(pal.accent, Color32::BLACK, 0.18),
            stroke: Stroke::NONE,
            hover_stroke: Stroke::NONE,
            text: pal.on_accent,
            font,
            min: egui::vec2(120.0, 32.0),
        },
    )
}

/// A red button for actions that can lose work, like ending a task.
pub fn danger_button(ui: &mut Ui, pal: &Palette, text: &str) -> Response {
    let font = egui::FontId::new(egui::TextStyle::Button.resolve(ui.style()).size, semibold());
    let base = blend(pal.error, Color32::BLACK, 0.12);
    painted_button(
        ui,
        text,
        true,
        ButtonLook {
            rest: base,
            hover: pal.error,
            press: blend(pal.error, Color32::BLACK, 0.25),
            stroke: Stroke::NONE,
            hover_stroke: Stroke::NONE,
            text: pal.on_accent,
            font,
            min: egui::vec2(96.0, 30.0),
        },
    )
}

/// A plain button with a soft hover fade and the pointing-hand cursor.
pub fn button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let w = ui.visuals().widgets.clone();
    let look = ButtonLook {
        rest: w.inactive.weak_bg_fill,
        hover: w.hovered.weak_bg_fill,
        press: w.active.weak_bg_fill,
        stroke: w.inactive.bg_stroke,
        hover_stroke: w.hovered.bg_stroke,
        text: ui.visuals().text_color(),
        font: egui::TextStyle::Button.resolve(ui.style()),
        min: egui::vec2(0.0, 30.0),
    };
    painted_button(ui, text, enabled, look)
}

/// The theme button: a sun while dark (it switches to light), a crescent
/// moon while light. The tooltip names the mode a click switches to.
pub fn theme_button(ui: &mut Ui, pal: &Palette, light: bool) -> Response {
    let label = if light {
        tr("Dark Mode")
    } else {
        tr("Light Mode")
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if ui.is_rect_visible(rect) {
        let w = &ui.visuals().widgets;
        let hover = fade(ui, response.id, response.hovered());
        let bg = blend(w.inactive.weak_bg_fill, w.hovered.weak_bg_fill, hover);
        let stroke = Stroke::new(
            w.inactive.bg_stroke.width,
            blend(w.inactive.bg_stroke.color, w.hovered.bg_stroke.color, hover),
        );
        ui.painter()
            .rect(rect, CornerRadius::same(5), bg, stroke, StrokeKind::Inside);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(5),
                ui.visuals().selection.stroke,
                StrokeKind::Outside,
            );
        }
        let c = rect.center();
        let ink = pal.text;
        if light {
            // Crescent: a full disc with an offset disc in the button colour cut out of it.
            ui.painter().circle_filled(c, 7.0, ink);
            ui.painter()
                .circle_filled(c + egui::vec2(3.6, -2.8), 6.0, bg);
        } else {
            ui.painter().circle_filled(c, 4.0, ink);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let dir = egui::vec2(a.cos(), a.sin());
                ui.painter()
                    .line_segment([c + dir * 6.2, c + dir * 8.6], Stroke::new(1.6, ink));
            }
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// A hyperlink that opens through Explorer so the browser does not inherit
/// RustCleanup's admin rights.
pub fn link(ui: &mut Ui, log: &Logger, text: &str, url: &str) {
    let response = ui.link(text).on_hover_text(url);
    if response.clicked()
        && let Err(e) = shell::open_unelevated(url)
    {
        log.error(e);
    }
}
