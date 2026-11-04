//! "Open on startup": an XDG autostart entry,
//! `~/.config/autostart/rustcleanup.desktop`.
//!
//! The desktop starts it at sign-in as the normal user, so RustCleanup opens
//! without root and offers the usual restart as root from the bottom bar.

use std::path::PathBuf;

use super::user;
use crate::events::Logger;
use crate::i18n::{tr, trf};

/// Passed to the program when the desktop starts it at sign-in.
pub const STARTUP_ARG: &str = "--startup";

fn entry_path() -> PathBuf {
    user::config_home().join("autostart").join("rustcleanup.desktop")
}

/// Quotes a path for the `Exec` key of a desktop entry.
fn exec_quote(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    // `%` starts a field code, so it is doubled.
    out.replace('%', "%%")
}

/// Undoes `exec_quote` for the first word of an `Exec` line.
fn exec_unquote(exec: &str) -> Option<String> {
    let exec = exec.trim().replace("%%", "%");
    if let Some(rest) = exec.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.push(chars.next()?),
                '"' => return Some(out),
                c => out.push(c),
            }
        }
        None
    } else {
        exec.split_whitespace().next().map(str::to_string)
    }
}

/// The program the autostart entry runs, or `None` if there is no entry.
pub fn registered_path() -> Option<String> {
    let text = std::fs::read_to_string(entry_path()).ok()?;
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    exec_unquote(exec)
}

fn current_exe() -> Result<String, String> {
    super::shell::app_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

fn entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=RustCleanup\n\
         Comment=Starts RustCleanup when you sign in.\n\
         Exec={} {STARTUP_ARG}\n\
         Icon=rustcleanup\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_quote(exe)
    )
}

fn write_entry() -> Result<(), String> {
    let path = entry_path();
    if let Some(dir) = path.parent() {
        user::create_dir(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, entry(&current_exe()?)).map_err(|e| e.to_string())?;
    user::give_to_user(&path);
    Ok(())
}

fn remove_entry() -> Result<(), String> {
    match std::fs::remove_file(entry_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Adds or removes the autostart entry.
pub fn set_enabled(log: &Logger, enable: bool) -> Result<(), String> {
    let result = if enable { write_entry() } else { remove_entry() };
    result.map_err(|e| trf("Startup task not changed: {0}", &[&e]))?;
    log.success(if enable {
        tr("RustCleanup will open when you sign in.")
    } else {
        tr("RustCleanup will no longer open when you sign in.")
    });
    Ok(())
}

/// Makes the entry match the saved choice. Run on launch, so the entry
/// follows the program if it was moved or replaced by an update.
pub fn sync(log: &Logger, wanted: bool) -> Result<(), String> {
    let registered = registered_path();
    let here = current_exe()?;
    match (wanted, registered) {
        (true, Some(path)) if path == here => Ok(()),
        (true, _) => set_enabled(log, true),
        (false, Some(_)) => set_enabled(log, false),
        (false, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_round_trips() {
        for p in ["/opt/Rust Cleanup/RustCleanup.AppImage", "/home/a/$x\"y%z", "/usr/bin/rc"] {
            assert_eq!(exec_unquote(&format!("{} --startup", exec_quote(p))).as_deref(), Some(p));
        }
    }
}
