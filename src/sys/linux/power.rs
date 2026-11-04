//! Low Power Mode on Linux: the `power-saver` profile from
//! power-profiles-daemon (or TuneD's compatible service) when the desktop
//! has one, plus, as root, a 60% CPU frequency cap and turbo boost off.
//! Everything it changes is written down first so turning it off puts the
//! machine back exactly as it was.

use std::fs;
use std::path::{Path, PathBuf};

use super::command::{run_hidden, which};
use super::privilege::is_elevated;
use super::registry::{self, Hive};
use crate::events::Logger;
use crate::i18n::{tr, trf};

const STATE_KEY: &str = r"Software\RustCleanup";
const ACTIVE_VALUE: &str = "LowPowerActive";
const PROFILE_VALUE: &str = "PreviousPowerProfile";
const CPU_VALUE: &str = "PreviousCpuLimits";
const SAVER: &str = "power-saver";
/// Share of the top frequency the CPU is held to.
const CAP_PERCENT: u64 = 60;

const INTEL_NO_TURBO: &str = "/sys/devices/system/cpu/intel_pstate/no_turbo";
const CPUFREQ_BOOST: &str = "/sys/devices/system/cpu/cpufreq/boost";

fn profiles_ctl(args: &[&str]) -> Option<String> {
    which("powerprofilesctl")?;
    let out = run_hidden("powerprofilesctl", args).ok()?;
    out.success.then(|| out.stdout.trim().to_string())
}

fn current_profile() -> Option<String> {
    profiles_ctl(&["get"]).filter(|p| !p.is_empty())
}

fn policies() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir("/sys/devices/system/cpu/cpufreq")
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.join("scaling_max_freq").exists())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

fn read_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Saves the CPU limits as `path=value;...` so they can be put back.
fn save_cpu_limits() -> String {
    let mut parts: Vec<String> = policies()
        .iter()
        .filter_map(|p| {
            let f = p.join("scaling_max_freq");
            Some(format!("{}={}", f.display(), read_u64(&f)?))
        })
        .collect();
    for f in [INTEL_NO_TURBO, CPUFREQ_BOOST] {
        if let Some(v) = read_u64(Path::new(f)) {
            parts.push(format!("{f}={v}"));
        }
    }
    parts.join(";")
}

fn restore_cpu_limits(saved: &str) {
    for (path, value) in saved.split(';').filter_map(|p| p.split_once('=')) {
        // Only ever write back to the files we read from.
        if path.starts_with("/sys/devices/system/cpu/") {
            let _ = fs::write(path, value);
        }
    }
}

/// Caps every CPU at `CAP_PERCENT` of its top speed and turns turbo off.
fn cap_cpu() -> bool {
    let mut any = false;
    for p in policies() {
        let (Some(max), Some(min)) = (
            read_u64(&p.join("cpuinfo_max_freq")),
            read_u64(&p.join("cpuinfo_min_freq")),
        ) else {
            continue;
        };
        let cap = (max * CAP_PERCENT / 100).max(min);
        any |= fs::write(p.join("scaling_max_freq"), cap.to_string()).is_ok();
    }
    any |= fs::write(INTEL_NO_TURBO, "1").is_ok();
    any |= fs::write(CPUFREQ_BOOST, "0").is_ok();
    any
}

/// True while RustCleanup's Low Power Mode is on.
pub fn low_power_active() -> bool {
    if registry::get_dword(Hive::CurrentUser, STATE_KEY, ACTIVE_VALUE) != Some(1) {
        return false;
    }
    // Someone may have picked another profile since; then it is off.
    match registry::get_string(Hive::CurrentUser, STATE_KEY, PROFILE_VALUE) {
        Some(_) => current_profile().is_none_or(|p| p == SAVER),
        None => true,
    }
}

/// Switches to the power-saver profile and, as root, caps the CPU.
pub fn enable_low_power(log: &Logger) -> Result<(), String> {
    let profile = current_profile();
    let mut done_profile = false;
    if let Some(current) = &profile {
        if current != SAVER {
            registry::set_string(Hive::CurrentUser, STATE_KEY, PROFILE_VALUE, current)?;
        }
        done_profile = profiles_ctl(&["set", SAVER]).is_some();
        if !done_profile {
            log.warn(trf("powerprofilesctl could not switch to {0}.", &[&SAVER]));
        }
    }

    let mut capped = false;
    if is_elevated() {
        // Never overwrite limits saved earlier and not yet put back.
        if registry::get_string(Hive::CurrentUser, STATE_KEY, CPU_VALUE).is_none() {
            registry::set_string(Hive::CurrentUser, STATE_KEY, CPU_VALUE, &save_cpu_limits())?;
        }
        capped = cap_cpu();
    }

    if !done_profile && !capped {
        return Err(tr(
            "Low Power Mode needs power-profiles-daemon, or RustCleanup running as root.",
        )
        .into());
    }
    registry::set_dword(Hive::CurrentUser, STATE_KEY, ACTIVE_VALUE, 1)?;
    if capped {
        log.success(tr(
            "Low Power Mode on: power saver, CPU capped at 60%, turbo boost off.",
        ));
    } else {
        log.success(tr("Low Power Mode on: power saver profile."));
    }
    Ok(())
}

/// Puts the profile and CPU limits back to what they were.
pub fn disable_low_power(log: &Logger) -> Result<(), String> {
    let previous = registry::get_string(Hive::CurrentUser, STATE_KEY, PROFILE_VALUE)
        .filter(|p| p != SAVER)
        .unwrap_or_else(|| "balanced".to_string());
    if current_profile().is_some() && profiles_ctl(&["set", &previous]).is_some() {
        log.info(trf("Restored power profile {0}.", &[&previous]));
    }
    if let Some(saved) = registry::get_string(Hive::CurrentUser, STATE_KEY, CPU_VALUE) {
        if is_elevated() {
            restore_cpu_limits(&saved);
            let _ = registry::delete_value(Hive::CurrentUser, STATE_KEY, CPU_VALUE);
        } else {
            log.warn(tr("The CPU limits can only be restored as root."));
        }
    }
    for v in [PROFILE_VALUE, ACTIVE_VALUE] {
        let _ = registry::delete_value(Hive::CurrentUser, STATE_KEY, v);
    }
    log.success(tr("Low Power Mode off."));
    Ok(())
}
