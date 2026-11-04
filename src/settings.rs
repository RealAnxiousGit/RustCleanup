//! User preferences, kept in `HKCU\Software\RustCleanup` on Windows and in
//! `~/.config/RustCleanup` on Linux.
//!
//! One process-wide copy behind a lock: every window and worker thread reads
//! the same values, and `update` writes changes straight back to the registry.

use std::sync::{LazyLock, RwLock};

use crate::i18n::{self, Lang, n_, os};
use crate::sys::registry::{self, Hive};

pub const KEY: &str = r"Software\RustCleanup";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => i18n::tr(os(n_("Same as Windows"), n_("Same as system"))),
            ThemeMode::Light => i18n::tr("Light"),
            ThemeMode::Dark => i18n::tr("Dark"),
        }
    }

    fn code(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

/// The interface font. Each is a font that ships with Windows 11; a missing
/// file falls back to Segoe UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontChoice {
    System,
    Arial,
    TimesNewRoman,
    LightUi,
    ComicSans,
    Verdana,
    Georgia,
    Tahoma,
    Calibri,
    Trebuchet,
    Consolas,
}

impl FontChoice {
    pub const ALL: [FontChoice; 11] = [
        FontChoice::System,
        FontChoice::Arial,
        FontChoice::TimesNewRoman,
        FontChoice::LightUi,
        FontChoice::ComicSans,
        FontChoice::Verdana,
        FontChoice::Georgia,
        FontChoice::Tahoma,
        FontChoice::Calibri,
        FontChoice::Trebuchet,
        FontChoice::Consolas,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FontChoice::System => i18n::tr(os(n_("System (Segoe UI)"), n_("System (sans-serif)"))),
            FontChoice::Arial => "Arial",
            FontChoice::TimesNewRoman => "Times New Roman",
            FontChoice::LightUi => {
                i18n::tr(os(n_("Light UI (Segoe UI Light)"), n_("Light UI (light sans-serif)")))
            }
            FontChoice::ComicSans => "Comic Sans MS",
            FontChoice::Verdana => "Verdana",
            FontChoice::Georgia => "Georgia",
            FontChoice::Tahoma => "Tahoma",
            FontChoice::Calibri => "Calibri",
            FontChoice::Trebuchet => "Trebuchet MS",
            FontChoice::Consolas => "Consolas",
        }
    }

    /// Linux: regular and bold fontconfig patterns. Fontconfig picks the
    /// closest installed face, so a missing font still gets a look-alike.
    #[cfg(not(windows))]
    pub fn files(self) -> (&'static str, &'static str) {
        match self {
            FontChoice::System => ("sans-serif", "sans-serif:bold"),
            FontChoice::Arial => ("Liberation Sans", "Liberation Sans:bold"),
            FontChoice::TimesNewRoman => ("Liberation Serif", "Liberation Serif:bold"),
            FontChoice::LightUi => ("sans-serif:light", "sans-serif"),
            FontChoice::ComicSans => ("Comic Neue,Comic Sans MS", "Comic Neue,Comic Sans MS:bold"),
            FontChoice::Verdana => ("DejaVu Sans,Verdana", "DejaVu Sans,Verdana:bold"),
            FontChoice::Georgia => ("Gelasio,Georgia,serif", "Gelasio,Georgia,serif:bold"),
            FontChoice::Tahoma => ("DejaVu Sans Condensed,Tahoma", "DejaVu Sans Condensed,Tahoma:bold"),
            FontChoice::Calibri => ("Carlito,Calibri", "Carlito,Calibri:bold"),
            FontChoice::Trebuchet => ("Ubuntu,Trebuchet MS,sans-serif", "Ubuntu,Trebuchet MS,sans-serif:bold"),
            FontChoice::Consolas => ("monospace", "monospace:bold"),
        }
    }

    /// Regular and bold files under `C:\Windows\Fonts`.
    #[cfg(windows)]
    pub fn files(self) -> (&'static str, &'static str) {
        match self {
            FontChoice::System => ("SegUIVar.ttf", "seguisb.ttf"),
            FontChoice::Arial => ("arial.ttf", "arialbd.ttf"),
            FontChoice::TimesNewRoman => ("times.ttf", "timesbd.ttf"),
            FontChoice::LightUi => ("segoeuil.ttf", "segoeui.ttf"),
            FontChoice::ComicSans => ("comic.ttf", "comicbd.ttf"),
            FontChoice::Verdana => ("verdana.ttf", "verdanab.ttf"),
            FontChoice::Georgia => ("georgia.ttf", "georgiab.ttf"),
            FontChoice::Tahoma => ("tahoma.ttf", "tahomabd.ttf"),
            FontChoice::Calibri => ("calibri.ttf", "calibrib.ttf"),
            FontChoice::Trebuchet => ("trebuc.ttf", "trebucbd.ttf"),
            FontChoice::Consolas => ("consola.ttf", "consolab.ttf"),
        }
    }

    fn code(self) -> &'static str {
        match self {
            FontChoice::System => "system",
            FontChoice::Arial => "arial",
            FontChoice::TimesNewRoman => "times",
            FontChoice::LightUi => "light",
            FontChoice::ComicSans => "comic",
            FontChoice::Verdana => "verdana",
            FontChoice::Georgia => "georgia",
            FontChoice::Tahoma => "tahoma",
            FontChoice::Calibri => "calibri",
            FontChoice::Trebuchet => "trebuchet",
            FontChoice::Consolas => "consolas",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub language: Lang,
    pub start_minimized: bool,
    /// What the user chose. The scheduled task is checked against it on launch.
    pub open_on_startup: bool,
    pub close_to_tray: bool,
    pub theme: ThemeMode,
    pub font: FontChoice,
    /// Interface size in percent.
    pub ui_scale: u16,
    pub notifications: bool,
    /// Download and install new releases on launch without asking.
    pub auto_update: bool,
    /// Ask about new releases (when `auto_update` is off). "Never Update" clears it.
    pub update_notify: bool,
    pub auto_clean: bool,
    /// Physical memory use, in percent, that triggers Auto-Clean RAM.
    pub auto_clean_percent: u8,
    pub purge_standby: bool,
    /// Direct3D 12 (wgpu) renderer instead of OpenGL (glow). Needs a restart.
    pub hardware_ui: bool,
}

pub const UI_SCALES: [u16; 6] = [90, 100, 110, 125, 150, 175];
pub const AUTO_CLEAN_RANGE: std::ops::RangeInclusive<u8> = 50..=95;

impl Settings {
    pub fn defaults() -> Self {
        Self {
            language: Lang::system_default(),
            start_minimized: false,
            open_on_startup: false,
            close_to_tray: true,
            theme: ThemeMode::System,
            font: FontChoice::System,
            ui_scale: 100,
            notifications: true,
            auto_update: false,
            update_notify: true,
            auto_clean: false,
            auto_clean_percent: 85,
            purge_standby: true,
            hardware_ui: false,
        }
    }

    fn load() -> Self {
        let d = Self::defaults();
        let flag = |name: &str, default: bool| {
            registry::get_dword(Hive::CurrentUser, KEY, name).map_or(default, |v| v != 0)
        };
        let number = |name: &str| registry::get_dword(Hive::CurrentUser, KEY, name);
        let text = |name: &str| registry::get_string(Hive::CurrentUser, KEY, name);
        Self {
            language: text("Language")
                .and_then(|c| Lang::from_code(&c))
                .unwrap_or(d.language),
            start_minimized: flag("StartMinimized", d.start_minimized),
            open_on_startup: flag("OpenOnStartup", d.open_on_startup),
            close_to_tray: flag("CloseToTray", d.close_to_tray),
            // v2 saved only "light" or "dark" here; no value means "follow Windows".
            theme: match text("Theme").as_deref() {
                Some("light") => ThemeMode::Light,
                Some("dark") => ThemeMode::Dark,
                _ => ThemeMode::System,
            },
            font: text("Font")
                .and_then(|c| FontChoice::ALL.into_iter().find(|f| f.code() == c))
                .unwrap_or(d.font),
            ui_scale: number("UiScale")
                .map(|v| v as u16)
                .filter(|v| UI_SCALES.contains(v))
                .unwrap_or(d.ui_scale),
            notifications: flag("Notifications", d.notifications),
            auto_update: flag("AutoUpdate", d.auto_update),
            update_notify: flag("UpdateNotify", d.update_notify),
            auto_clean: flag("AutoCleanRam", d.auto_clean),
            auto_clean_percent: number("AutoCleanPercent")
                .map(|v| v.clamp(50, 95) as u8)
                .unwrap_or(d.auto_clean_percent),
            purge_standby: flag("PurgeStandby", d.purge_standby),
            hardware_ui: flag("HardwareUi", d.hardware_ui),
        }
    }

    /// Writes every value that differs from `old`.
    fn save(&self, old: &Settings) {
        let dword = |name: &str, v: u32| {
            let _ = registry::set_dword(Hive::CurrentUser, KEY, name, v);
        };
        let flag = |name: &str, new: bool, old: bool| {
            if new != old {
                dword(name, new as u32);
            }
        };
        if self.language != old.language {
            let _ = registry::set_string(Hive::CurrentUser, KEY, "Language", self.language.code());
        }
        if self.theme != old.theme {
            let _ = registry::set_string(Hive::CurrentUser, KEY, "Theme", self.theme.code());
        }
        if self.font != old.font {
            let _ = registry::set_string(Hive::CurrentUser, KEY, "Font", self.font.code());
        }
        if self.ui_scale != old.ui_scale {
            dword("UiScale", self.ui_scale as u32);
        }
        if self.auto_clean_percent != old.auto_clean_percent {
            dword("AutoCleanPercent", self.auto_clean_percent as u32);
        }
        flag("StartMinimized", self.start_minimized, old.start_minimized);
        flag("OpenOnStartup", self.open_on_startup, old.open_on_startup);
        flag("CloseToTray", self.close_to_tray, old.close_to_tray);
        flag("Notifications", self.notifications, old.notifications);
        flag("AutoUpdate", self.auto_update, old.auto_update);
        flag("UpdateNotify", self.update_notify, old.update_notify);
        flag("AutoCleanRam", self.auto_clean, old.auto_clean);
        flag("PurgeStandby", self.purge_standby, old.purge_standby);
        flag("HardwareUi", self.hardware_ui, old.hardware_ui);
    }
}

static SETTINGS: LazyLock<RwLock<Settings>> = LazyLock::new(|| {
    let s = Settings::load();
    i18n::set_language(s.language);
    RwLock::new(s)
});

/// The current settings (a copy).
pub fn get() -> Settings {
    *SETTINGS.read().unwrap_or_else(|e| e.into_inner())
}

/// Changes settings and saves whatever changed. Returns the new values.
pub fn update(change: impl FnOnce(&mut Settings)) -> Settings {
    let mut guard = SETTINGS.write().unwrap_or_else(|e| e.into_inner());
    let old = *guard;
    change(&mut guard);
    guard.save(&old);
    if guard.language != old.language {
        i18n::set_language(guard.language);
    }
    *guard
}

/// Puts every setting back to its default, keeping the startup choice
/// (that one changes a scheduled task and has its own switch).
pub fn reset() -> Settings {
    update(|s| {
        let startup = s.open_on_startup;
        *s = Settings::defaults();
        s.open_on_startup = startup;
    })
}

/// Remembers which version ran, and says whether the last launch was an
/// older one, i.e. RustCleanup was just updated. `updated_hint` covers
/// versions before 3.0.0, which did not save their version.
pub fn note_version(current: &str, updated_hint: bool) -> bool {
    let last = registry::get_string(Hive::CurrentUser, KEY, "LastVersion");
    if last.as_deref() != Some(current) {
        let _ = registry::set_string(Hive::CurrentUser, KEY, "LastVersion", current);
    }
    match last {
        Some(last) => crate::sys::updater::is_newer(current, &last),
        None => updated_hint,
    }
}
