//! System Tools dropdown: quick launchers for built-in Windows utilities.

use std::os::windows::process::CommandExt;
use std::process::Command;

use super::command::{CREATE_NO_WINDOW, system32};
use crate::i18n::n_;

/// A launchable Windows tool.
pub struct Tool {
    pub label: &'static str,
    pub description: &'static str,
    exe: &'static str,
    args: &'static [&'static str],
    /// Hide the launcher's own console (used for the PowerShell elevation hop).
    hidden: bool,
}

pub const TOOLS: [Tool; 8] = [
    Tool {
        label: n_("Power Config"),
        description: n_("Power Options control panel"),
        exe: "control.exe",
        args: &["powercfg.cpl"],
        hidden: false,
    },
    Tool {
        label: n_("Disk Cleanup"),
        description: n_("Remove temporary and system files"),
        exe: "cleanmgr.exe",
        args: &[],
        hidden: false,
    },
    Tool {
        label: n_("Disk Management"),
        description: n_("Partitions and volumes"),
        exe: "mmc.exe",
        args: &["diskmgmt.msc"],
        hidden: false,
    },
    Tool {
        label: n_("Performance Monitor"),
        description: n_("Counters and data collectors"),
        exe: "perfmon.exe",
        args: &[],
        hidden: false,
    },
    Tool {
        label: n_("Task Manager"),
        description: n_("Processes, performance, startup apps"),
        exe: "taskmgr.exe",
        args: &[],
        hidden: false,
    },
    Tool {
        label: n_("Resource Monitor"),
        description: n_("Live CPU, memory, disk and network"),
        exe: "resmon.exe",
        args: &[],
        hidden: false,
    },
    // Launched through Explorer so the prompt runs as the normal user, not
    // with RustCleanup's elevated token.
    Tool {
        label: n_("Command Prompt"),
        description: n_("Standard (non-admin) prompt"),
        exe: r"..\explorer.exe",
        args: &["cmd.exe"],
        hidden: false,
    },
    Tool {
        label: n_("Command Prompt [Admin]"),
        description: n_("Elevated prompt"),
        exe: r"WindowsPowerShell\v1.0\powershell.exe",
        args: &[
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "Start-Process cmd -Verb RunAs",
        ],
        hidden: true,
    },
];

impl Tool {
    /// Launches the tool without waiting for it.
    pub fn launch(&self) -> Result<(), String> {
        let mut cmd = Command::new(system32(self.exe));
        cmd.args(self.args);
        if self.hidden {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            cmd.current_dir(home);
        }
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("could not start {}: {e}", self.label))
    }
}
