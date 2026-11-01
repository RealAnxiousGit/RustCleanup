//! No Forced Updates: notify-only Windows Update, no automatic reboots,
//! and an optional pause.

use super::format::utc_iso_in_days;
use super::registry::{self, Hive};
use crate::events::Logger;

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
    registry::get_dword(
        Hive::LocalMachine,
        WU_AU_POLICY,
        "NoAutoRebootWithLoggedOnUsers",
    ) == Some(1)
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
        registry::set_dword(
            Hive::LocalMachine,
            WU_AU_POLICY,
            "NoAutoRebootWithLoggedOnUsers",
            1,
        )?;
        registry::set_dword(
            Hive::LocalMachine,
            WU_AU_POLICY,
            "AlwaysAutoRebootAtScheduledTime",
            0,
        )?;
        // 2 = "Notify before download and install": nothing installs without consent.
        registry::set_dword(Hive::LocalMachine, WU_AU_POLICY, "AUOptions", 2)?;
        registry::set_dword(Hive::LocalMachine, WU_AU_POLICY, "NoAutoUpdate", 0)?;
        // Don't push feature-update reboot deadlines.
        registry::set_dword(
            Hive::LocalMachine,
            WU_POLICY,
            "SetAutoRestartNotificationDisable",
            1,
        )?;
        log.info("Windows Update set to notify only, with no automatic reboots.");

        if pause {
            let start = utc_iso_in_days(0);
            let end = utc_iso_in_days(PAUSE_DAYS);
            for name in PAUSE_VALUES {
                let value = if name.contains("Start") { &start } else { &end };
                registry::set_string(Hive::LocalMachine, WU_UX_SETTINGS, name, value)?;
            }
            log.info(format!(
                "Updates paused until {end} (the maximum Windows allows)."
            ));
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
        registry::delete_value(
            Hive::LocalMachine,
            WU_POLICY,
            "SetAutoRestartNotificationDisable",
        )?;
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
