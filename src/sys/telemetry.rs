//! Disable Telemetry: data-collection policies, services and scheduled tasks.

use super::command::run_hidden;
use super::registry::{self, Hive};
use super::tasks::{self, Outcome};
use crate::events::Logger;
use crate::i18n::{tr, trf};

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
        log.info(tr("Telemetry policy AllowTelemetry=0 written."));
    } else {
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_POLICY, "AllowTelemetry")?;
        registry::delete_value(Hive::LocalMachine, DATA_COLLECTION_SYSTEM, "AllowTelemetry")?;
        registry::delete_value(
            Hive::LocalMachine,
            DATA_COLLECTION_POLICY,
            "DoNotShowFeedbackNotifications",
        )?;
        log.info(tr("Telemetry policies removed."));
    }

    // 2. Services.
    for (service, default_start) in TELEMETRY_SERVICES {
        let start = if disable { "disabled" } else { default_start };
        match run_hidden("sc.exe", &["config", service, "start=", start]) {
            Ok(o) if o.success => {
                log.info(trf("Service {0}: startup set to {1}.", &[&service, &start]))
            }
            Ok(o) => log.warn(trf("Service {0}: {1}", &[&service, &o.summary()])),
            Err(e) => log.warn(trf("Service {0}: {1}", &[&service, &e])),
        }
        let action = if disable { "stop" } else { "start" };
        if disable || service == "DiagTrack" {
            match run_hidden("sc.exe", &[action, service]) {
                Ok(o) if o.success => {
                    log.info(trf("Service {0}: {1} requested.", &[&service, &action]))
                }
                // 1062 = not started, 1056 = already running: both fine.
                Ok(o) if matches!(o.code, Some(1062) | Some(1056)) => {}
                Ok(o) => log.warn(trf(
                    "Service {0} {1}: {2}",
                    &[&service, &action, &o.summary()],
                )),
                Err(e) => log.warn(trf("Service {0} {1}: {2}", &[&service, &action, &e])),
            }
        }
    }

    // 3. Scheduled tasks, through Task Scheduler directly. A missing task
    //    never stops the others.
    match tasks::set_enabled_all(&TELEMETRY_TASKS, !disable) {
        Ok(results) => {
            let (mut ok, mut missing) = (0, 0);
            for (task, outcome) in results {
                match outcome {
                    Outcome::Changed => ok += 1,
                    Outcome::Missing => missing += 1,
                    Outcome::Failed(msg) => log.warn(trf("Task {0}: {1}", &[&task, &msg])),
                }
            }
            log.info(if disable {
                trf(
                    "{0} telemetry scheduled tasks disabled ({1} not present on this Windows build).",
                    &[&ok, &missing],
                )
            } else {
                trf(
                    "{0} telemetry scheduled tasks enabled ({1} not present on this Windows build).",
                    &[&ok, &missing],
                )
            });
        }
        Err(e) => log.warn(trf("Scheduled tasks not changed: {0}", &[&e])),
    }

    log.success(if disable {
        tr("Telemetry disabled.")
    } else {
        tr("Telemetry restored to Windows defaults.")
    });
    Ok(())
}
