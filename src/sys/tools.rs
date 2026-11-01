//! System Tools dropdown: quick launchers for built-in Windows utilities.

use std::os::windows::process::CommandExt;
use std::process::Command;

use super::util::{CREATE_NO_WINDOW, system32};

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
    Tool { label: "Power Config", description: "Power Options control panel", exe: "control.exe", args: &["powercfg.cpl"], hidden: false },
    Tool { label: "Disk Cleanup", description: "Remove temporary and system files", exe: "cleanmgr.exe", args: &[], hidden: false },
    Tool { label: "Disk Management", description: "Partitions and volumes", exe: "mmc.exe", args: &["diskmgmt.msc"], hidden: false },
    Tool { label: "Performance Monitor", description: "Counters and data collectors", exe: "perfmon.exe", args: &[], hidden: false },
    Tool { label: "Task Manager", description: "Processes, performance, startup apps", exe: "taskmgr.exe", args: &[], hidden: false },
    Tool { label: "Resource Monitor", description: "Live CPU, memory, disk and network", exe: "resmon.exe", args: &[], hidden: false },
    // Launched through Explorer so the prompt runs as the normal user, not
    // with RustCleanup's elevated token.
    Tool { label: "Command Prompt", description: "Standard (non-admin) prompt", exe: r"..\explorer.exe", args: &["cmd.exe"], hidden: false },
    Tool {
        label: "Command Prompt [Admin]",
        description: "Elevated prompt",
        exe: r"WindowsPowerShell\v1.0\powershell.exe",
        args: &["-NoProfile", "-WindowStyle", "Hidden", "-Command", "Start-Process cmd -Verb RunAs"],
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
        cmd.spawn().map(|_| ()).map_err(|e| format!("could not start {}: {e}", self.label))
    }
}
