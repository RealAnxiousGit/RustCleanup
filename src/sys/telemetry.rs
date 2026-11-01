//! Disable Telemetry: data-collection policies, services and scheduled tasks.

use super::command::{run_hidden, run_powershell};
use super::registry::{self, Hive};
use crate::events::Logger;

const DATA_COLLECTION_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\DataCollection";
const DATA_COLLECTION_SYSTEM: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection";

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
        registry::set_dword(
            Hive::LocalMachine,
            DATA_COLLECTION_POLICY,
            "AllowTelemetry",
            0,
        )?;
        registry::set_dword(
            Hive::LocalMachine,
            DATA_COLLECTION_SYSTEM,
            "AllowTelemetry",
            0,
        )?;
        registry::set_dword(
            Hive::LocalMachine,
            DATA_COLLECTION_POLICY,
            "DoNotShowFeedbackNotifications",
            1,
        )?;
        log.info("Telemetry policy AllowTelemetry=0 written.");
    } else {
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_POLICY, "AllowTelemetry")?;
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_SYSTEM, "AllowTelemetry")?;
        registry::delete_value(
            Hive::LocalMachine,
            DATA_COLLECTION_POLICY,
            "DoNotShowFeedbackNotifications",
        )?;
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
    let verb = if disable {
        "Disable-ScheduledTask"
    } else {
        "Enable-ScheduledTask"
    };
    let list = TELEMETRY_TASKS
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
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

    log.success(if disable {
        "Telemetry disabled."
    } else {
        "Telemetry restored to Windows defaults."
    });
    Ok(())
}
