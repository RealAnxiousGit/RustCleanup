//! Colours, fonts and the light/dark switch with its fade.
//! The reasoning behind each choice is in DESIGN.md.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Shadow, Stroke, Visuals,
};

use crate::sys::registry::{self, Hive};
use crate::sys::shell;

/// Every colour the UI uses. Panels read status colours from here so both
/// themes stay consistent.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    pub terminal_bg: Color32,
    pub terminal_text: Color32,
    pub meter_track: Color32,
    pub meter_fill: Color32,
}

/// Graphite. Measured contrast on `bg`: text 13.7:1, muted 7.0:1 (WCAG AA needs 4.5:1).
pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x1B, 0x1C, 0x1E),
    surface: Color32::from_rgb(0x22, 0x23, 0x26),
    raised: Color32::from_rgb(0x2A, 0x2B, 0x2F),
    border: Color32::from_rgb(0x3A, 0x3C, 0x41),
    text: Color32::from_rgb(0xE8, 0xE6, 0xE3),
    muted: Color32::from_rgb(0xA9, 0xA6, 0xA0),
    accent: Color32::from_rgb(0xB4, 0x53, 0x2A),
    accent_hover: Color32::from_rgb(0xC2, 0x5E, 0x33),
    on_accent: Color32::WHITE,
    ok: Color32::from_rgb(0x7C, 0xC4, 0x8C),
    warn: Color32::from_rgb(0xE0, 0xB0, 0x52),
    error: Color32::from_rgb(0xEE, 0x7D, 0x72),
    terminal_bg: Color32::from_rgb(0x12, 0x12, 0x13),
    terminal_text: Color32::from_rgb(0xD6, 0xD4, 0xCF),
    meter_track: Color32::from_rgb(0x34, 0x36, 0x3A),
    meter_fill: Color32::from_rgb(0xC9, 0xC6, 0xC0),
};

/// Warm paper. Measured contrast on `bg`: text 14.8:1, muted 6.3:1.
pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF3, 0xF1, 0xEC),
    surface: Color32::from_rgb(0xFA, 0xF9, 0xF6),
    raised: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    border: Color32::from_rgb(0xD6, 0xD2, 0xC9),
    text: Color32::from_rgb(0x1F, 0x1E, 0x1C),
    muted: Color32::from_rgb(0x5C, 0x58, 0x51),
    accent: Color32::from_rgb(0xB4, 0x53, 0x2A),
    accent_hover: Color32::from_rgb(0x9E, 0x47, 0x22),
    on_accent: Color32::WHITE,
    ok: Color32::from_rgb(0x2B, 0x6E, 0x3D),
    warn: Color32::from_rgb(0x85, 0x55, 0x00),
    error: Color32::from_rgb(0xAE, 0x2A, 0x1F),
    terminal_bg: Color32::from_rgb(0x1E, 0x1E, 0x1E),
    terminal_text: Color32::from_rgb(0xD6, 0xD4, 0xCF),
    meter_track: Color32::from_rgb(0xE2, 0xDE, 0xD5),
    meter_fill: Color32::from_rgb(0x4A, 0x47, 0x42),
};

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

impl Palette {
    /// Blend between two palettes; `t = 0` is `a`, `t = 1` is `b`.
    pub fn lerp(a: &Palette, b: &Palette, t: f32) -> Palette {
        let c = |x: Color32, y: Color32| lerp_color(x, y, t);
        Palette {
            bg: c(a.bg, b.bg),
            surface: c(a.surface, b.surface),
            raised: c(a.raised, b.raised),
            border: c(a.border, b.border),
            text: c(a.text, b.text),
            muted: c(a.muted, b.muted),
            accent: c(a.accent, b.accent),
            accent_hover: c(a.accent_hover, b.accent_hover),
            on_accent: c(a.on_accent, b.on_accent),
            ok: c(a.ok, b.ok),
            warn: c(a.warn, b.warn),
            error: c(a.error, b.error),
            terminal_bg: c(a.terminal_bg, b.terminal_bg),
            terminal_text: c(a.terminal_text, b.terminal_text),
            meter_track: c(a.meter_track, b.meter_track),
            meter_fill: c(a.meter_fill, b.meter_fill),
        }
    }

    /// egui visuals built from this palette.
    fn visuals(&self, dark: bool) -> Visuals {
        let mut v = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let radius = CornerRadius::same(4);
        v.panel_fill = self.bg;
        v.window_fill = self.bg;
        v.faint_bg_color = self.surface;
        v.extreme_bg_color = self.raised;
        v.code_bg_color = self.surface;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = radius;
        // One small shadow, only on popups (combo boxes, tooltips), which really are above the page.
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(40),
        };
        v.hyperlink_color = if dark {
            Color32::from_rgb(0xE0, 0x8A, 0x63)
        } else {
            Color32::from_rgb(0x9E, 0x47, 0x22)
        };
        v.warn_fg_color = self.warn;
        v.error_fg_color = self.error;
        v.override_text_color = None;
        v.weak_text_color = Some(self.muted);
        v.striped = true;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.surface;
        w.noninteractive.weak_bg_fill = self.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        for (state, fill) in [
            (&mut w.inactive, self.raised),
            (&mut w.hovered, lerp_color(self.raised, self.text, 0.08)),
            (&mut w.active, lerp_color(self.raised, self.text, 0.16)),
            (&mut w.open, self.raised),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, self.border);
            state.fg_stroke = Stroke::new(1.0, self.text);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
        w.hovered.bg_stroke = Stroke::new(1.0, lerp_color(self.border, self.text, 0.35));
        w.noninteractive.corner_radius = radius;

        // Selection doubles as the keyboard focus ring, so it must be clearly visible.
        v.selection.bg_fill = lerp_color(self.accent, self.bg, 0.55);
        v.selection.stroke = Stroke::new(
            2.0,
            if dark {
                Color32::from_rgb(0xE0, 0x8A, 0x63)
            } else {
                self.accent
            },
        );
        v
    }
}

const THEME_KEY: &str = r"Software\RustCleanup";

/// The current theme, the fade between themes, and what has been applied.
pub struct Theme {
    pub light: bool,
    animate: bool,
    applied: Option<f32>,
    pub palette: Palette,
}

impl Theme {
    /// Uses the saved choice, or follows Windows' app theme on first launch.
    pub fn load() -> Self {
        let light = match registry::get_string(Hive::CurrentUser, THEME_KEY, "Theme").as_deref() {
            Some("light") => true,
            Some("dark") => false,
            _ => shell::system_prefers_light(),
        };
        Self {
            light,
            animate: shell::animations_enabled(),
            applied: None,
            palette: if light { LIGHT } else { DARK },
        }
    }

    pub fn toggle(&mut self) {
        self.light = !self.light;
        self.animate = shell::animations_enabled();
        let _ = registry::set_string(
            Hive::CurrentUser,
            THEME_KEY,
            "Theme",
            if self.light { "light" } else { "dark" },
        );
    }

    /// Advances the fade and re-applies visuals only when the colours changed.
    pub fn update(&mut self, ctx: &egui::Context) {
        let target = if self.light { 1.0 } else { 0.0 };
        let duration = if self.animate { 0.28 } else { 0.0 };
        let t = ctx.animate_value_with_time(egui::Id::unique("theme_fade"), target, duration);
        if self.applied == Some(t) {
            return;
        }
        self.applied = Some(t);
        self.palette = Palette::lerp(&DARK, &LIGHT, t);
        let visuals = self.palette.visuals(t < 0.5);
        // egui also gives every native window's title bar the matching theme.
        ctx.set_theme(if self.light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });
        ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
        ctx.set_visuals_of(egui::Theme::Light, visuals);
    }
}

/// Font families used beyond egui's two defaults.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Loads Windows' own UI and console fonts, keeping egui's bundled fonts as
/// fallbacks so missing glyphs (or a missing file) still render.
pub fn install_fonts(ctx: &egui::Context) {
    let fonts_dir = crate::sys::command::windows_dir().join("Fonts");
    let read = |file: &str| {
        std::fs::read(fonts_dir.join(file))
            .ok()
            .map(|b| Arc::new(FontData::from_owned(b)))
    };

    let mut defs = FontDefinitions::default();
    let fallback_prop = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    if let Some(data) = read("SegUIVar.ttf") {
        defs.font_data.insert("segoe".into(), data);
        if let Some(list) = defs.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "segoe".into());
        }
    }
    let mut semibold_list = fallback_prop.clone();
    if let Some(data) = read("seguisb.ttf") {
        defs.font_data.insert("segoe_semibold".into(), data);
        semibold_list.insert(0, "segoe_semibold".into());
    } else if defs.font_data.contains_key("segoe") {
        semibold_list.insert(0, "segoe".into());
    }
    defs.families.insert(semibold(), semibold_list);

    if let Some(data) = read("consola.ttf") {
        defs.font_data.insert("consolas".into(), data);
        if let Some(list) = defs.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, "consolas".into());
        }
    }
    ctx.set_fonts(defs);

    ctx.all_styles_mut(|style| {
        use egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::new(17.0, semibold())),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.interaction.tooltip_delay = 0.25;
    });
}
