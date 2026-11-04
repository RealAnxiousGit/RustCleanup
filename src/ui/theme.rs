//! Colours, fonts and the light/dark switch with its fade.
//! The reasoning behind each choice is in DESIGN.md.

use std::sync::{Arc, OnceLock, RwLock};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Shadow, Stroke, Visuals,
};

use crate::i18n::Lang;
use crate::settings::{self, FontChoice, ThemeMode};
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

/// The palette every window draws with. Detached windows paint on their own
/// schedule, so they read it from here rather than from the main window.
static CURRENT: RwLock<Palette> = RwLock::new(DARK);

pub fn palette() -> Palette {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The current theme, the fade between themes, and what has been applied.
pub struct Theme {
    pub light: bool,
    animate: bool,
    applied: Option<f32>,
    pub palette: Palette,
}

impl Theme {
    pub fn load() -> Self {
        let light = match settings::get().theme {
            ThemeMode::Light => true,
            ThemeMode::Dark => false,
            ThemeMode::System => shell::system_prefers_light(),
        };
        let palette = if light { LIGHT } else { DARK };
        *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = palette;
        Self {
            light,
            animate: shell::animations_enabled(),
            applied: None,
            palette,
        }
    }

    /// The theme button: switches to the other theme and remembers it.
    pub fn toggle(&mut self) {
        let mode = if self.light {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        settings::update(|s| s.theme = mode);
    }

    /// Follows the setting (and Windows, in "Same as Windows" mode), advances
    /// the fade, and re-applies visuals only when the colours changed.
    /// Returns true while the colours are changing.
    pub fn update(&mut self, ctx: &egui::Context) -> bool {
        let light = match settings::get().theme {
            ThemeMode::Light => true,
            ThemeMode::Dark => false,
            ThemeMode::System => match ctx.system_theme() {
                Some(t) => t == egui::Theme::Light,
                None => self.light,
            },
        };
        if light != self.light {
            self.light = light;
            self.animate = shell::animations_enabled();
        }
        let target = if self.light { 1.0 } else { 0.0 };
        let duration = if self.animate { 0.28 } else { 0.0 };
        let t = ctx.animate_value_with_time(egui::Id::unique("theme_fade"), target, duration);
        if self.applied == Some(t) {
            return false;
        }
        self.applied = Some(t);
        self.palette = Palette::lerp(&DARK, &LIGHT, t);
        *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = self.palette;
        let visuals = self.palette.visuals(t < 0.5);
        // egui also gives every native window's title bar the matching theme.
        ctx.set_theme(if self.light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });
        ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
        ctx.set_visuals_of(egui::Theme::Light, visuals);
        true
    }
}

/// Font families used beyond egui's two defaults.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Maps a font file into memory for the rest of the process. The CJK fonts
/// are 13 to 19 MB each; mapped, only the pages for glyphs actually drawn
/// are read, and they are shared with every other program using the font.
#[cfg(windows)]
fn map_font(path: &std::path::Path) -> Option<&'static [u8]> {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GetFileSizeEx, OPEN_EXISTING,
    };
    use windows::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY,
    };
    use windows::core::HSTRING;
    // SAFETY: a read-only mapping of a system font file. The view is never
    // unmapped, which is what makes the 'static lifetime sound.
    unsafe {
        let file = CreateFileW(
            &HSTRING::from(path.as_os_str()),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;
        let mut size = 0i64;
        let sized = GetFileSizeEx(file, &mut size).is_ok();
        let mapping = CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, None);
        let _ = CloseHandle(file);
        let mapping = mapping.ok()?;
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        let _ = CloseHandle(mapping);
        if view.Value.is_null() || !sized || size <= 0 {
            return None;
        }
        Some(std::slice::from_raw_parts(
            view.Value as *const u8,
            size as usize,
        ))
    }
}

/// Linux: maps a font file read-only for the life of the process, so the
/// big CJK collections cost page cache rather than private memory.
#[cfg(not(windows))]
fn map_font(path: &std::path::Path) -> Option<&'static [u8]> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len() as usize;
    if size == 0 {
        return None;
    }
    // SAFETY: a fresh read-only private mapping of a file we just opened;
    // it is never unmapped, so the slice stays valid for 'static.
    unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        );
        if ptr == libc::MAP_FAILED {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr as *const u8, size))
    }
}

/// Linux: the file and face index fontconfig picks for `pattern`.
#[cfg(not(windows))]
fn fc_match(pattern: &str) -> Option<(std::path::PathBuf, u32)> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}\n%{index}", pattern])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let file = lines.next()?.trim();
    if file.is_empty() {
        return None;
    }
    let index = lines.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    Some((file.into(), index))
}

/// Linux: the first installed face that covers `lang`, preferring a CJK
/// collection such as Noto Sans CJK.
#[cfg(not(windows))]
fn fc_lang(lang: &str) -> Option<(std::path::PathBuf, u32)> {
    let out = std::process::Command::new("fc-list")
        .args([format!(":lang={lang}").as_str(), "-f", "%{file}|%{index}|%{family}\n"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut faces: Vec<(String, u32, String)> = text
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '|');
            let file = parts.next()?.to_string();
            let index = parts.next()?.parse().ok()?;
            Some((file, index, parts.next().unwrap_or("").to_string()))
        })
        .collect();
    let rank = |(file, _, family): &(String, u32, String)| {
        let regular = !file.contains("Bold") && !file.contains("Light") && !file.contains("Thin");
        let sans = family.contains("Sans");
        let cjk = family.contains("CJK") || family.contains("Gothic") || family.contains("Hei");
        (!(cjk && sans), !cjk, !regular, file.clone())
    };
    faces.sort_by_key(rank);
    // In a CJK collection the face for this language is the one whose
    // family names it (Noto Sans CJK JP, Noto Sans CJK SC).
    let tag = if lang == "ja" { "JP" } else { "SC" };
    let best = faces.first()?.0.clone();
    let face = faces
        .iter()
        .filter(|f| f.0 == best)
        .find(|f| f.2.contains(tag))
        .or_else(|| faces.first())?;
    Some((face.0.clone().into(), face.1))
}

/// How far CJK glyphs are moved down, as a fraction of the font size.
const CJK_Y_OFFSET: f32 = 0.22;

/// Japanese and Chinese fonts, mapped once. Each is (name, bytes, face index).
fn cjk_fonts() -> &'static [(&'static str, &'static [u8], u32)] {
    static FONTS: OnceLock<Vec<(&'static str, &'static [u8], u32)>> = OnceLock::new();
    #[cfg(windows)]
    let init = || {
        let dir = crate::sys::command::windows_dir().join("Fonts");
        [
            ("cjk_ja", ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"]),
            ("cjk_zh", ["msyh.ttc", "simsun.ttc", "msyh.ttf"]),
        ]
        .into_iter()
        .filter_map(|(name, files)| {
            files
                .iter()
                .find_map(|f| map_font(&dir.join(f)))
                .map(|bytes| (name, bytes, 0))
        })
        .collect()
    };
    #[cfg(not(windows))]
    let init = || {
        [("cjk_ja", "ja"), ("cjk_zh", "zh-cn")]
            .into_iter()
            .filter_map(|(name, lang)| {
                let (file, index) = fc_lang(lang)?;
                map_font(&file).map(|bytes| (name, bytes, index))
            })
            .collect()
    };
    FONTS.get_or_init(init)
}

/// Loads the Windows UI and console fonts, keeping egui's bundled fonts as
/// fallbacks so missing glyphs (or a missing file) still render. Japanese or
/// Chinese comes first among the CJK fallbacks depending on `lang`, because
/// the two draw many shared characters differently. `font` is the face
/// picked in Settings; Segoe UI stays behind it for any glyph it lacks.
pub fn install_fonts(ctx: &egui::Context, lang: Lang, font: FontChoice) {
    #[cfg(windows)]
    let fonts_dir = crate::sys::command::windows_dir().join("Fonts");
    #[cfg(windows)]
    let read = |file: &str| {
        std::fs::read(fonts_dir.join(file))
            .ok()
            .map(|b| Arc::new(FontData::from_owned(b)))
    };
    // Linux: `file` is a fontconfig pattern.
    #[cfg(not(windows))]
    let read = |pattern: &str| {
        let (path, index) = fc_match(pattern)?;
        let mut data = FontData::from_owned(std::fs::read(path).ok()?);
        data.index = index;
        Some(Arc::new(data))
    };
    let (ui_face, ui_bold, mono_face) = if cfg!(windows) {
        ("SegUIVar.ttf", "seguisb.ttf", "consola.ttf")
    } else {
        ("sans-serif", "sans-serif:weight=demibold", "monospace")
    };

    let mut defs = FontDefinitions::default();
    let fallback_prop = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let (regular, bold) = font.files();
    let mut prop_list = fallback_prop.clone();
    let mut semibold_list = fallback_prop.clone();
    // The picked face first, then Segoe UI, then egui's own fonts.
    let mut faces = vec![("segoe", ui_face, "segoe_semibold", ui_bold)];
    if font != FontChoice::System {
        faces.insert(0, ("chosen", regular, "chosen_bold", bold));
    }
    for (at, (name, file, bold_name, bold_file)) in faces.into_iter().enumerate() {
        let Some(data) = read(file) else { continue };
        defs.font_data.insert(name.into(), data);
        let at = at.min(prop_list.len());
        prop_list.insert(at, name.into());
        if let Some(data) = read(bold_file) {
            defs.font_data.insert(bold_name.into(), data);
            semibold_list.insert(at.min(semibold_list.len()), bold_name.into());
        } else {
            semibold_list.insert(at.min(semibold_list.len()), name.into());
        }
    }
    // A face that failed to load leaves fewer entries; drop duplicates.
    let mut seen = std::collections::HashSet::new();
    prop_list.retain(|n| seen.insert(n.clone()));
    seen.clear();
    semibold_list.retain(|n| seen.insert(n.clone()));
    defs.families.insert(FontFamily::Proportional, prop_list);
    defs.families.insert(semibold(), semibold_list);

    if let Some(data) = read(mono_face) {
        defs.font_data.insert("consolas".into(), data);
        if let Some(list) = defs.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, "consolas".into());
        }
    }

    let mut cjk: Vec<_> = cjk_fonts().to_vec();
    if lang == Lang::Zh {
        cjk.reverse();
    }
    for (name, bytes, index) in cjk {
        let mut data = FontData::from_static(bytes);
        data.index = index;
        // Yu Gothic and Microsoft YaHei draw higher than Segoe UI on the same
        // line; without this, mixed lines like "Windows の設定" look jumbled.
        data.tweak.y_offset_factor = CJK_Y_OFFSET;
        defs.font_data.insert(name.into(), Arc::new(data));
        for family in [FontFamily::Proportional, FontFamily::Monospace, semibold()] {
            if let Some(list) = defs.families.get_mut(&family) {
                list.push(name.into());
            }
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
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.interact_size.y = 30.0;
        style.interaction.tooltip_delay = 0.25;
        // Hover and open/close fades, quick enough not to feel slow.
        style.animation_time = 0.12;
        // Wheel scrolling glides a short way instead of jumping.
        style.scroll_animation = egui::style::ScrollAnimation {
            points_per_second: 2200.0,
            duration: egui::Rangef::new(0.08, 0.18),
        };
        // Scroll bars still float, but get a strip of their own at the right
        // edge, so a hovered (wider) bar never covers buttons near it.
        style.spacing.scroll.floating_allocated_width =
            style.spacing.scroll.bar_width + style.spacing.scroll.bar_inner_margin;
    });
}
