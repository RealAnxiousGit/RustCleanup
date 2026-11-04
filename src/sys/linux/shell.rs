//! Desktop helpers: opening links and programs as the desktop user, the
//! running program's own path, and the desktop's theme and animation
//! settings.

use std::path::PathBuf;
use std::process::Stdio;

use super::command::which;
use super::user::{self, user_command};

/// The file to start again for a restart or at sign-in: the AppImage when
/// running from one (its mount point changes every run), else this binary.
pub fn app_exe() -> std::io::Result<PathBuf> {
    match std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()) {
        Some(p) => Ok(p.into()),
        None => std::env::current_exe(),
    }
}

/// Starts a program without waiting for it. A thread reaps it when it
/// ends, so finished helpers do not linger as zombies.
pub fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<()> {
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Opens a link, file or folder with the desktop user's default app, never
/// as root.
pub fn open_unelevated(target: &str) -> Result<(), String> {
    let opener = ["xdg-open", "gio", "kde-open", "exo-open"]
        .into_iter()
        .find(|p| which(p).is_some())
        .ok_or("xdg-open is not installed")?;
    let mut cmd = user_command(which(opener).unwrap_or_else(|| opener.into()));
    if opener == "gio" {
        cmd.arg("open");
    }
    cmd.arg(target)
        .current_dir(&user::desktop_user().home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    spawn_detached(&mut cmd).map_err(|e| format!("could not open {target}: {e}"))
}

/// Native window lookups are not needed on Linux: winit brings the window
/// back on its own, so these are stand-ins with the Windows signatures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowId(pub isize);

pub fn find_own_window(_title: &str) -> Option<WindowId> {
    None
}

pub fn show_window(_id: WindowId) {}

/// A `gsettings get` as the desktop user, without quotes.
fn gsetting(schema: &str, key: &str) -> Option<String> {
    which("gsettings")?;
    let out = user_command("gsettings")
        .args(["get", schema, key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string())
}

/// KDE's settings, from `~/.config/kdeglobals`.
fn kde_setting(section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(user::config_home().join("kdeglobals")).ok()?;
    let mut in_section = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == format!("[{section}]");
        } else if in_section && let Some((k, v)) = line.split_once('=') && k.trim() == key {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// False when the desktop has animations turned off, in which case the
/// theme switches instantly instead of fading.
pub fn animations_enabled() -> bool {
    if let Some(v) = gsetting("org.gnome.desktop.interface", "enable-animations") {
        return v != "false";
    }
    if let Some(v) = kde_setting("KDE", "AnimationDurationFactor") {
        return v.parse::<f32>().map_or(true, |f| f > 0.0);
    }
    true
}

/// True if the desktop is set to a light theme.
pub fn system_prefers_light() -> bool {
    if let Some(v) = gsetting("org.gnome.desktop.interface", "color-scheme") {
        if v == "prefer-dark" {
            return false;
        }
        if v == "prefer-light" {
            return true;
        }
        if let Some(theme) = gsetting("org.gnome.desktop.interface", "gtk-theme") {
            return !theme.to_ascii_lowercase().contains("dark");
        }
    }
    if let Some(scheme) = kde_setting("General", "ColorScheme") {
        return !scheme.to_ascii_lowercase().contains("dark");
    }
    if let Ok(theme) = std::env::var("GTK_THEME") {
        return !theme.to_ascii_lowercase().contains("dark");
    }
    // No desktop setting to go by: RustCleanup's own default is dark.
    false
}
