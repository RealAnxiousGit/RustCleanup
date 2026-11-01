//! Thin wrappers around `winreg` that turn errors into readable messages.

use std::io::ErrorKind;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};

/// Registry hive a value lives in.
#[derive(Clone, Copy, Debug)]
pub enum Hive {
    LocalMachine,
    CurrentUser,
}

impl Hive {
    fn key(self) -> RegKey {
        match self {
            Hive::LocalMachine => RegKey::predef(HKEY_LOCAL_MACHINE),
            Hive::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Hive::LocalMachine => "HKLM",
            Hive::CurrentUser => "HKCU",
        }
    }
}

fn describe(hive: Hive, path: &str, name: &str) -> String {
    format!("{}\\{path}\\{name}", hive.prefix())
}

/// Creates the key if needed and writes a REG_DWORD.
pub fn set_dword(hive: Hive, path: &str, name: &str, value: u32) -> Result<(), String> {
    let (key, _) = hive
        .key()
        .create_subkey(path)
        .map_err(|e| format!("cannot open {}: {e}", describe(hive, path, "")))?;
    key.set_value(name, &value)
        .map_err(|e| format!("cannot write {}: {e}", describe(hive, path, name)))
}

/// Creates the key if needed and writes a REG_SZ.
pub fn set_string(hive: Hive, path: &str, name: &str, value: &str) -> Result<(), String> {
    let (key, _) = hive
        .key()
        .create_subkey(path)
        .map_err(|e| format!("cannot open {}: {e}", describe(hive, path, "")))?;
    key.set_value(name, &value)
        .map_err(|e| format!("cannot write {}: {e}", describe(hive, path, name)))
}

/// Reads a REG_DWORD, returning `None` if the key or value is missing.
pub fn get_dword(hive: Hive, path: &str, name: &str) -> Option<u32> {
    hive.key()
        .open_subkey_with_flags(path, KEY_READ)
        .ok()?
        .get_value(name)
        .ok()
}

/// Reads a REG_SZ, returning `None` if the key or value is missing.
pub fn get_string(hive: Hive, path: &str, name: &str) -> Option<String> {
    hive.key()
        .open_subkey_with_flags(path, KEY_READ)
        .ok()?
        .get_value(name)
        .ok()
}

/// Deletes a value. A value or key that is already gone counts as success,
/// so reverting a tweak is idempotent.
pub fn delete_value(hive: Hive, path: &str, name: &str) -> Result<(), String> {
    let key = match hive
        .key()
        .open_subkey_with_flags(path, KEY_READ | KEY_WRITE)
    {
        Ok(k) => k,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot open {}: {e}", describe(hive, path, ""))),
    };
    match key.delete_value(name) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot delete {}: {e}", describe(hive, path, name))),
    }
}
