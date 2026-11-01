//! Small custom widgets shared by every panel.

use eframe::egui::{
    self, Color32, CornerRadius, Response, RichText, Role, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo,
};

use super::theme::{Palette, semibold};
use crate::events::Logger;
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
        let mut fill = lerp(pal.meter_track, pal.accent, t);
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

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A thin usage bar. `warn` switches the fill to the warning colour, which
/// only happens when usage is genuinely high.
pub fn meter(ui: &mut Ui, pal: &Palette, fraction: f32, warn: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), Sense::hover());
    let radius = CornerRadius::same(2);
    ui.painter().rect_filled(rect, radius, pal.meter_track);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter()
        .rect_filled(fill, radius, if warn { pal.warn } else { pal.meter_fill });
}

/// The accent-filled button. Used for exactly one action per screen.
pub fn primary_button(ui: &mut Ui, pal: &Palette, text: &str, enabled: bool) -> Response {
    let fill = if enabled {
        pal.accent
    } else {
        pal.accent.gamma_multiply(0.45)
    };
    let button = egui::Button::new(RichText::new(text).color(pal.on_accent).family(semibold()))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(4))
        .min_size(egui::vec2(120.0, 32.0));
    let response = ui.add_enabled(enabled, button);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A plain button with the pointing-hand cursor.
pub fn button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text).min_size(egui::vec2(0.0, 28.0)),
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The theme button: a sun while dark (it switches to light), a crescent
/// moon while light. The tooltip names the mode a click switches to.
pub fn theme_button(ui: &mut Ui, pal: &Palette, light: bool) -> Response {
    let label = if light { "Dark Mode" } else { "Light Mode" };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            CornerRadius::same(4),
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(4),
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
                .circle_filled(c + egui::vec2(3.6, -2.8), 6.0, visuals.weak_bg_fill);
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
