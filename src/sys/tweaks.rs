//! Registry/service tweaks: Disable Copilot, Disable Telemetry and
//! No Forced Updates. Every tweak can be detected and fully reverted.

use std::time::Duration;

use super::registry::{self, Hive};
use super::util::{run_hidden, run_powershell};
use crate::events::Logger;

// ---------------------------------------------------------------------------
// Copilot
// ---------------------------------------------------------------------------

const COPILOT_POLICY: &str = r"Software\Policies\Microsoft\Windows\WindowsCopilot";
const EXPLORER_ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

/// True if the Copilot "turn off" policy is set for this user or machine.
pub fn copilot_disabled() -> bool {
    [Hive::CurrentUser, Hive::LocalMachine]
        .iter()
        .any(|&h| registry::get_dword(h, COPILOT_POLICY, "TurnOffWindowsCopilot") == Some(1))
}

/// Applies or reverts the Copilot policy, then optionally restarts Explorer
/// so the taskbar picks it up immediately.
pub fn set_copilot_disabled(log: &Logger, disable: bool, restart_shell: bool) -> Result<(), String> {
    if disable {
        // Policy for both the current user and the whole machine.
        registry::set_dword(Hive::CurrentUser, COPILOT_POLICY, "TurnOffWindowsCopilot", 1)?;
        registry::set_dword(Hive::LocalMachine, COPILOT_POLICY, "TurnOffWindowsCopilot", 1)?;
        // Hide the taskbar button on builds that still have one.
        registry::set_dword(Hive::CurrentUser, EXPLORER_ADVANCED, "ShowCopilotButton", 0)?;
        log.info("Copilot policy TurnOffWindowsCopilot=1 written to HKCU and HKLM.");
    } else {
        registry::delete_value(Hive::CurrentUser, COPILOT_POLICY, "TurnOffWindowsCopilot")?;
        registry::delete_value(Hive::LocalMachine, COPILOT_POLICY, "TurnOffWindowsCopilot")?;
        registry::delete_value(Hive::CurrentUser, EXPLORER_ADVANCED, "ShowCopilotButton")?;
        log.info("Copilot policy removed; Windows defaults restored.");
    }
    if restart_shell {
        restart_explorer(log)?;
    }
    log.success(if disable { "Copilot disabled." } else { "Copilot re-enabled." });
    Ok(())
}

/// Restarts explorer.exe so shell policies apply without signing out.
///
/// We only kill Explorer and let Winlogon relaunch it (`AutoRestartShell`).
/// Starting it ourselves would make the shell inherit our elevated token,
/// so that is only a fallback if Winlogon does not bring it back.
pub fn restart_explorer(log: &Logger) -> Result<(), String> {
    log.info("Restarting Explorer to apply the change...");
    let out = run_hidden("taskkill.exe", &["/F", "/IM", "explorer.exe"])?;
    if !out.success {
        log.warn(format!("taskkill explorer.exe: {}", out.summary()));
    }
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        if super::processes::is_running("explorer.exe") {
            log.info("Explorer restarted.");
            return Ok(());
        }
    }
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    std::process::Command::new(format!("{windir}\\explorer.exe"))
        .spawn()
        .map_err(|e| format!("Explorer did not restart and could not be started: {e}"))?;
    log.warn("Explorer was started manually; sign out and back in if the taskbar misbehaves.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

const DATA_COLLECTION_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\DataCollection";
const DATA_COLLECTION_SYSTEM: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection";

/// Telemetry services: (name, start type to restore when re-enabling).
const TELEMETRY_SERVICES: [(&str, &str); 2] = [
    ("DiagTrack", "auto"),          // Connected User Experiences and Telemetry
    ("dmwappushservice", "demand"), // WAP Push Message Routing (telemetry routing)
];

/// Scheduled tasks that collect and upload usage data. Names vary between
/// Windows builds, so both old and new names are listed; whichever are
/// missing on this machine are simply reported as not present.
const TELEMETRY_TASKS: [&str; 9] = [
    r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
    r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser Exp",
    r"\Microsoft\Windows\Application Experience\MareBackup",
    r"\Microsoft\Windows\Application Experience\ProgramDataUpdater",
    r"\Microsoft\Windows\Autochk\Proxy",
    r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
    r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeip",
    r"\Microsoft\Windows\DiskDiagnostic\Microsoft-Windows-DiskDiagnosticDataCollector",
    r"\Microsoft\Windows\Feedback\Siuf\DmClient",
];

/// True if the telemetry policy is at its lowest level.
pub fn telemetry_disabled() -> bool {
    registry::get_dword(Hive::LocalMachine, DATA_COLLECTION_POLICY, "AllowTelemetry") == Some(0)
}

/// Disables or restores telemetry: policies, services and scheduled tasks.
///
/// Individual service/task failures are logged as warnings rather than
/// aborting, because some of them do not exist on every Windows edition.
pub fn set_telemetry_disabled(log: &Logger, disable: bool) -> Result<(), String> {
    // 1. Policies. Note: Home/Pro treat 0 ("Security") as 1 ("Required"),
    //    which is still the minimum those editions allow.
    if disable {
        registry::set_dword(Hive::LocalMachine, DATA_COLLECTION_POLICY, "AllowTelemetry", 0)?;
        registry::set_dword(Hive::LocalMachine, DATA_COLLECTION_SYSTEM, "AllowTelemetry", 0)?;
        registry::set_dword(Hive::LocalMachine, DATA_COLLECTION_POLICY, "DoNotShowFeedbackNotifications", 1)?;
        log.info("Telemetry policy AllowTelemetry=0 written.");
    } else {
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_POLICY, "AllowTelemetry")?;
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_SYSTEM, "AllowTelemetry")?;
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_POLICY, "DoNotShowFeedbackNotifications")?;
        log.info("Telemetry policies removed.");
    }

    // 2. Services.
    for (service, default_start) in TELEMETRY_SERVICES {
        let start = if disable { "disabled" } else { default_start };
        match run_hidden("sc.exe", &["config", service, "start=", start]) {
            Ok(o) if o.success => log.info(format!("Service {service}: startup set to {start}.")),
            Ok(o) => log.warn(format!("Service {service}: {}", o.summary())),
            Err(e) => log.warn(format!("Service {service}: {e}")),
        }
        let action = if disable { "stop" } else { "start" };
        if disable || service == "DiagTrack" {
            match run_hidden("sc.exe", &[action, service]) {
                Ok(o) if o.success => log.info(format!("Service {service}: {action} requested.")),
                // 1062 = not started, 1056 = already running: both fine.
                Ok(o) if matches!(o.code, Some(1062) | Some(1056)) => {}
                Ok(o) => log.warn(format!("Service {service} {action}: {}", o.summary())),
                Err(e) => log.warn(format!("Service {service} {action}: {e}")),
            }
        }
    }

    // 3. Scheduled tasks, in one silent PowerShell run. Each task prints a
    //    status line we can parse, so a missing task never stops the others.
    let verb = if disable { "Disable-ScheduledTask" } else { "Enable-ScheduledTask" };
    let list = TELEMETRY_TASKS.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(",");
    // Existence is checked first (rather than parsing error text) so this
    // works the same on every display language.
    let script = format!(
        r#"foreach($t in @({list})) {{
             $i=$t.LastIndexOf('\'); $p=$t.Substring(0,$i+1); $n=$t.Substring($i+1);
             $task=Get-ScheduledTask -TaskPath $p -TaskName $n -ErrorAction SilentlyContinue;
             if (-not $task) {{ "MISSING|$t" }}
             else {{ try {{ $task | {verb} -ErrorAction Stop | Out-Null; "OK|$t" }}
                    catch {{ "FAIL|$t|$($_.Exception.Message)" }} }} }}"#
    );
    match run_powershell(&script) {
        Ok(out) => {
            let (mut ok, mut missing) = (0, 0);
            for line in out.stdout.lines() {
                let parts: Vec<&str> = line.trim().splitn(3, '|').collect();
                match parts.as_slice() {
                    ["OK", _] => ok += 1,
                    ["MISSING", _] => missing += 1,
                    ["FAIL", task, msg] => log.warn(format!("Task {task}: {}", msg.trim())),
                    _ => {}
                }
            }
            log.info(format!(
                "{ok} telemetry scheduled tasks {} ({missing} not present on this Windows build).",
                if disable { "disabled" } else { "enabled" }
            ));
        }
        Err(e) => log.warn(format!("Scheduled tasks not changed: {e}")),
    }

    log.success(if disable { "Telemetry disabled." } else { "Telemetry restored to Windows defaults." });
    Ok(())
}

// ---------------------------------------------------------------------------
// No forced updates
// ---------------------------------------------------------------------------

const WU_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate";
const WU_AU_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU";
const WU_UX_SETTINGS: &str = r"SOFTWARE\Microsoft\WindowsUpdate\UX\Settings";

/// Windows allows pausing for at most 5 weeks.
pub const PAUSE_DAYS: u64 = 35;

const PAUSE_VALUES: [&str; 6] = [
    "PauseUpdatesStartTime",
    "PauseUpdatesExpiryTime",
    "PauseFeatureUpdatesStartTime",
    "PauseFeatureUpdatesEndTime",
    "PauseQualityUpdatesStartTime",
    "PauseQualityUpdatesEndTime",
];

/// True if our "no automatic reboot" policy is in place.
pub fn forced_updates_blocked() -> bool {
    registry::get_dword(Hive::LocalMachine, WU_AU_POLICY, "NoAutoRebootWithLoggedOnUsers") == Some(1)
}

/// When updates are paused until, if a pause is active.
pub fn paused_until() -> Option<String> {
    registry::get_string(Hive::LocalMachine, WU_UX_SETTINGS, "PauseUpdatesExpiryTime")
}

/// Stops Windows Update from rebooting or installing on its own, and
/// optionally pauses updates for 5 weeks. Reverting removes everything.
///
/// Security updates are not blocked forever: the pause expires and the
/// user can still install updates manually from Settings at any time.
pub fn set_forced_updates_blocked(log: &Logger, block: bool, pause: bool) -> Result<(), String> {
    if block {
        // Never reboot automatically while someone is signed in.
        registry::set_dword(Hive::LocalMachine, WU_AU_POLICY, "NoAutoRebootWithLoggedOnUsers", 1)?;
        registry::set_dword(Hive::LocalMachine, WU_AU_POLICY, "AlwaysAutoRebootAtScheduledTime", 0)?;
        // 2 = "Notify before download and install": nothing installs without consent.
        registry::set_dword(Hive::LocalMachine, WU_AU_POLICY, "AUOptions", 2)?;
        registry::set_dword(Hive::LocalMachine, WU_AU_POLICY, "NoAutoUpdate", 0)?;
        // Don't push feature-update reboot deadlines.
        registry::set_dword(Hive::LocalMachine, WU_POLICY, "SetAutoRestartNotificationDisable", 1)?;
        log.info("Windows Update set to notify only, with no automatic reboots.");

        if pause {
            let start = super::util::utc_iso_in_days(0);
            let end = super::util::utc_iso_in_days(PAUSE_DAYS);
            for name in PAUSE_VALUES {
                let value = if name.contains("Start") { &start } else { &end };
                registry::set_string(Hive::LocalMachine, WU_UX_SETTINGS, name, value)?;
            }
            log.info(format!("Updates paused until {end} (the maximum Windows allows)."));
        }
    } else {
        for name in [
            "NoAutoRebootWithLoggedOnUsers",
            "AlwaysAutoRebootAtScheduledTime",
            "AUOptions",
            "NoAutoUpdate",
        ] {
            registry::delete_value(Hive::LocalMachine, WU_AU_POLICY, name)?;
        }
        registry::delete_value(Hive::LocalMachine, WU_POLICY, "SetAutoRestartNotificationDisable")?;
        for name in PAUSE_VALUES {
            registry::delete_value(Hive::LocalMachine, WU_UX_SETTINGS, name)?;
        }
        log.info("Windows Update policies and pause removed.");
    }
    log.success(if block {
        "Forced updates and automatic reboots are now blocked."
    } else {
        "Windows Update restored to default behaviour."
    });
    Ok(())
}
