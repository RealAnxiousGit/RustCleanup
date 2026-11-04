//! Settings storage. Windows keeps RustCleanup's settings in the registry;
//! on Linux they live in `~/.config/RustCleanup/settings.conf`, one
//! `key=value` per line, behind the same functions so the rest of the app
//! does not care which system it runs on.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use super::user;

/// Where a value lives. Both map to the desktop user's settings file.
#[derive(Clone, Copy, Debug)]
pub enum Hive {
    #[allow(dead_code)]
    LocalMachine,
    CurrentUser,
}

static LOCK: Mutex<()> = Mutex::new(());

/// The folder that holds RustCleanup's settings for the desktop user.
pub fn config_dir() -> PathBuf {
    user::config_home().join("RustCleanup")
}

fn file() -> PathBuf {
    config_dir().join("settings.conf")
}

fn key(path: &str, name: &str) -> String {
    let path = path.rsplit('\\').next().unwrap_or(path);
    if path == "RustCleanup" || path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

fn load() -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(file()).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), unescape(v)))
        .collect()
}

fn save(values: &BTreeMap<String, String>) -> Result<(), String> {
    let dir = config_dir();
    user::create_dir(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut text = String::from("# RustCleanup settings\n");
    for (k, v) in values {
        text.push_str(&format!("{k}={}\n", escape(v)));
    }
    let path = file();
    let tmp = path.with_extension("conf.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    user::give_to_user(&tmp);
    std::fs::rename(&tmp, &path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn set(path: &str, name: &str, value: String) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut values = load();
    values.insert(key(path, name), value);
    save(&values)
}

fn get(path: &str, name: &str) -> Option<String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load().remove(&key(path, name))
}

/// Writes a whole number.
pub fn set_dword(_hive: Hive, path: &str, name: &str, value: u32) -> Result<(), String> {
    set(path, name, value.to_string())
}

/// Writes a string.
pub fn set_string(_hive: Hive, path: &str, name: &str, value: &str) -> Result<(), String> {
    set(path, name, value.to_string())
}

/// Reads a whole number, `None` when missing.
pub fn get_dword(_hive: Hive, path: &str, name: &str) -> Option<u32> {
    get(path, name)?.trim().parse().ok()
}

/// Reads a string, `None` when missing.
pub fn get_string(_hive: Hive, path: &str, name: &str) -> Option<String> {
    get(path, name)
}

/// Deletes a value. One that is already gone counts as success.
pub fn delete_value(_hive: Hive, path: &str, name: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut values = load();
    if values.remove(&key(path, name)).is_some() {
        save(&values)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_round_trip() {
        for s in ["plain", "a\\b", "line\nbreak", "end\\"] {
            assert_eq!(unescape(&escape(s)), s);
        }
        assert_eq!(key(r"Software\RustCleanup", "Theme"), "Theme");
    }
}
