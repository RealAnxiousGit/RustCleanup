//! Manage Storage on Linux: opens the drive in the desktop's disk usage
//! viewer (GNOME's Disk Usage Analyzer, KDE's Filelight, QDirStat), or in
//! the file manager when none is installed. Always as the desktop user.

use std::process::Stdio;

use super::command::which;
use super::shell::open_unelevated;
use super::user::user_command;

const VIEWERS: [&str; 3] = ["baobab", "filelight", "qdirstat"];

/// Opens `volume` (a mount point) for a look at what fills it.
pub fn open_drive(volume: &str) -> Result<(), String> {
    if let Some(viewer) = VIEWERS.into_iter().find_map(which) {
        let mut cmd = user_command(&viewer);
        cmd.arg(volume)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        return super::shell::spawn_detached(&mut cmd)
            .map_err(|e| format!("could not start {}: {e}", viewer.display()));
    }
    open_unelevated(volume)
}
