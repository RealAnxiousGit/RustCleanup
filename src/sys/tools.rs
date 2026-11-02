//! System Tools dropdown: quick launchers for built-in Windows utilities.

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use super::command::system32;
use super::shell;
use crate::i18n::n_;

/// A launchable Windows tool.
pub struct Tool {
    pub label: &'static str,
    pub description: &'static str,
    exe: &'static str,
    args: &'static [&'static str],
    /// Start as the signed-in user instead of with RustCleanup's admin token.
    unelevated: bool,
}

pub const TOOLS: [Tool; 11] = [
    Tool {
        label: n_("Power Config"),
        description: n_("Power Options control panel"),
        exe: "control.exe",
        args: &["powercfg.cpl"],
        unelevated: false,
    },
    Tool {
        label: n_("Disk Cleanup"),
        description: n_("Remove temporary and system files"),
        exe: "cleanmgr.exe",
        args: &[],
        unelevated: false,
    },
    Tool {
        label: n_("Disk Management"),
        description: n_("Partitions and volumes"),
        exe: "mmc.exe",
        args: &["diskmgmt.msc"],
        unelevated: false,
    },
    Tool {
        label: n_("Performance Monitor"),
        description: n_("Counters and data collectors"),
        exe: "perfmon.exe",
        args: &[],
        unelevated: false,
    },
    Tool {
        label: n_("Task Manager"),
        description: n_("Processes, performance, startup apps"),
        exe: "taskmgr.exe",
        args: &[],
        unelevated: false,
    },
    Tool {
        label: n_("Resource Monitor"),
        description: n_("Live CPU, memory, disk and network"),
        exe: "resmon.exe",
        args: &[],
        unelevated: false,
    },
    // Settings pages open by URI, like from the Start menu.
    Tool {
        label: n_("Windows Security"),
        description: n_("Virus protection, firewall and device security"),
        exe: "windowsdefender:",
        args: &[],
        unelevated: true,
    },
    Tool {
        label: n_("Device Manager"),
        description: n_("Hardware and drivers"),
        exe: "mmc.exe",
        args: &["devmgmt.msc"],
        unelevated: false,
    },
    Tool {
        label: n_("Settings"),
        description: n_("Windows Settings"),
        exe: "ms-settings:",
        args: &[],
        unelevated: true,
    },
    Tool {
        label: n_("Command Prompt"),
        description: n_("Standard (non-admin) prompt"),
        exe: "cmd.exe",
        args: &[],
        unelevated: true,
    },
    // RustCleanup is already elevated, so a prompt it starts directly is too.
    Tool {
        label: n_("Command Prompt [Admin]"),
        description: n_("Elevated prompt"),
        exe: "cmd.exe",
        args: &[],
        unelevated: false,
    },
];

impl Tool {
    /// Launches the tool without waiting for it. Both kinds start in the
    /// user's profile folder, like a prompt opened from the Start menu.
    pub fn launch(&self) -> Result<(), String> {
        if self.exe.ends_with(':') {
            return shell::open_unelevated(self.exe)
                .map_err(|e| format!("could not open {}: {e}", self.label));
        }
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let exe = system32(self.exe);
        if self.unelevated {
            return shell::run_unelevated(&exe.to_string_lossy(), &self.args.join(" "), &home)
                .map_err(|e| format!("could not start {}: {e}", self.label));
        }
        // ShellExecute rather than CreateProcess, so a console tool gets its
        // own console window even though RustCleanup has none.
        // SAFETY: every string outlives the call.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(exe.as_os_str()),
                &HSTRING::from(self.args.join(" ")),
                &HSTRING::from(home),
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success; lower ones are SE_ERR_* codes.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(format!(
                "could not start {}: error {}",
                self.label, result.0 as usize
            ))
        }
    }
}
