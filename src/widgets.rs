//! Small custom widgets shared by the UI.

use eframe::egui::{self, Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui};

/// Accent colour used across the app (rust orange).
pub const ACCENT: Color32 = Color32::from_rgb(240, 110, 50);
pub const OK_GREEN: Color32 = Color32::from_rgb(90, 200, 120);
pub const WARN_AMBER: Color32 = Color32::from_rgb(240, 180, 60);
pub const ERR_RED: Color32 = Color32::from_rgb(235, 90, 90);

/// An iOS-style on/off switch. Returns a response that is `clicked()` when toggled.
pub fn toggle_switch(ui: &mut Ui, on: bool, enabled: bool) -> Response {
    let size = egui::vec2(2.2, 1.15) * ui.spacing().interact_size.y;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, on);
        let radius = rect.height() / 2.0;
        let off_fill = ui.visuals().widgets.inactive.bg_fill;
        let mut fill = lerp_color(off_fill, ACCENT, t);
        if !enabled {
            fill = fill.gamma_multiply(0.5);
        }
        let stroke = if response.hovered() && enabled {
            Stroke::new(1.0, ui.visuals().widgets.hovered.fg_stroke.color)
        } else {
            Stroke::NONE
        };
        ui.painter().rect(rect, CornerRadius::same(radius as u8), fill, stroke, StrokeKind::Inside);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), radius * 0.78, Color32::WHITE);
    }
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A rounded card container with a title and optional subtitle.
pub fn card<R>(ui: &mut Ui, title: &str, subtitle: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    let visuals = ui.visuals();
    egui::Frame::new()
        .fill(visuals.faint_bg_color)
        .stroke(Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).size(17.0).strong());
            if !subtitle.is_empty() {
                ui.label(egui::RichText::new(subtitle).small().weak());
            }
            ui.add_space(8.0);
            body(ui)
        })
        .inner
}

/// A primary (accent-filled) button.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let button = egui::Button::new(egui::RichText::new(text).strong().color(Color32::WHITE))
        .fill(if enabled { ACCENT } else { ACCENT.gamma_multiply(0.4) })
        .min_size(egui::vec2(130.0, 32.0));
    ui.add_enabled(enabled, button)
}
