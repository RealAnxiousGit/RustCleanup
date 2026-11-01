//! Low Power Mode: a custom power scheme (based on Power Saver) that caps
//! the CPU and disables turbo boost to cut load and heat, with a one-click
//! revert to whatever scheme was active before.

use super::command::run_hidden;
use super::registry::{self, Hive};
use crate::events::Logger;

/// GUID of the scheme RustCleanup creates. Fixed so we can always find it.
pub const LOW_POWER_GUID: &str = "6c1e0d6a-5b2f-4c3e-9a51-7a2d5e0c11aa";
const POWER_SAVER_GUID: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";
const BALANCED_GUID: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

const STATE_KEY: &str = r"Software\RustCleanup";
const PREVIOUS_VALUE: &str = "PreviousPowerScheme";

/// (subgroup alias, setting alias, AC value, DC value) applied to our scheme.
const SETTINGS: [(&str, &str, u32, u32); 4] = [
    ("SUB_PROCESSOR", "PROCTHROTTLEMAX", 60, 50), // max processor state %
    ("SUB_PROCESSOR", "PROCTHROTTLEMIN", 5, 5),   // min processor state %
    ("SUB_PROCESSOR", "PERFBOOSTMODE", 0, 0),     // turbo boost off
    ("SUB_PROCESSOR", "SYSCOOLPOL", 0, 0),        // passive cooling: slow CPU before fans
];

/// Finds the first GUID (8-4-4-4-12 hex) in a line of powercfg output.
fn parse_guid(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .find(|tok| {
            let parts: Vec<&str> = tok.split('-').collect();
            parts.len() == 5 && parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
        })
        .map(str::to_ascii_lowercase)
}

/// GUID of the currently active power scheme.
pub fn active_scheme() -> Result<String, String> {
    let out = run_hidden("powercfg.exe", &["/getactivescheme"])?;
    if !out.success {
        return Err(format!("powercfg /getactivescheme: {}", out.summary()));
    }
    parse_guid(&out.stdout).ok_or_else(|| "could not parse the active power scheme".into())
}

/// The machine's Power Saver scheme. Newer Windows 11 builds ship it under
/// a different GUID than the classic one, so look it up by name first.
fn power_saver_scheme() -> Option<String> {
    let out = run_hidden("powercfg.exe", &["/list"]).ok()?;
    out.stdout
        .lines()
        .find(|l| l.to_ascii_lowercase().contains("saver"))
        .and_then(parse_guid)
        .filter(|g| g != LOW_POWER_GUID)
}

fn scheme_exists(guid: &str) -> bool {
    run_hidden("powercfg.exe", &["/list"])
        .map(|o| o.stdout.to_ascii_lowercase().contains(guid))
        .unwrap_or(false)
}

/// True if our low-power scheme is the active one.
pub fn low_power_active() -> bool {
    active_scheme()
        .map(|g| g == LOW_POWER_GUID)
        .unwrap_or(false)
}

fn powercfg(log: &Logger, args: &[&str]) -> Result<(), String> {
    let out = run_hidden("powercfg.exe", args)?;
    if out.success {
        Ok(())
    } else {
        let msg = format!("powercfg {}: {}", args.join(" "), out.summary());
        log.warn(&msg);
        Err(msg)
    }
}

/// Switches to (creating if needed) the RustCleanup low-power scheme.
pub fn enable_low_power(log: &Logger) -> Result<(), String> {
    let current = active_scheme()?;
    if current != LOW_POWER_GUID {
        // Remember what to go back to.
        registry::set_string(Hive::CurrentUser, STATE_KEY, PREVIOUS_VALUE, &current)?;
    }

    if !scheme_exists(LOW_POWER_GUID) {
        // Power Saver is hidden on many Windows 11 machines but can still be
        // duplicated; fall back to Balanced and tune it ourselves.
        let mut bases: Vec<String> = power_saver_scheme().into_iter().collect();
        bases.extend([POWER_SAVER_GUID.to_string(), BALANCED_GUID.to_string()]);
        bases.dedup();
        let created = bases
            .iter()
            .filter(|base| scheme_exists(base))
            .any(|base| powercfg(log, &["/duplicatescheme", base, LOW_POWER_GUID]).is_ok());
        if !created {
            return Err("could not create the low-power scheme".into());
        }
        let _ = powercfg(
            log,
            &[
                "/changename",
                LOW_POWER_GUID,
                "RustCleanup Low Power",
                "Created by RustCleanup: capped CPU, no turbo boost, passive cooling.",
            ],
        );
        log.info("Created the \"RustCleanup Low Power\" power scheme.");
    }

    for (sub, setting, ac, dc) in SETTINGS {
        // Failures (e.g. a setting this CPU doesn't expose) are logged by powercfg().
        let _ = powercfg(
            log,
            &[
                "/setacvalueindex",
                LOW_POWER_GUID,
                sub,
                setting,
                &ac.to_string(),
            ],
        );
        let _ = powercfg(
            log,
            &[
                "/setdcvalueindex",
                LOW_POWER_GUID,
                sub,
                setting,
                &dc.to_string(),
            ],
        );
    }

    powercfg(log, &["/setactive", LOW_POWER_GUID])?;
    log.success(
        "Low Power Mode on: CPU capped at 60% (plugged in) / 50% (battery), turbo boost off.",
    );
    Ok(())
}

/// Restores the scheme that was active before Low Power Mode and removes ours.
pub fn disable_low_power(log: &Logger) -> Result<(), String> {
    let previous = registry::get_string(Hive::CurrentUser, STATE_KEY, PREVIOUS_VALUE)
        .filter(|g| g != LOW_POWER_GUID && scheme_exists(g))
        .unwrap_or_else(|| BALANCED_GUID.to_string());

    powercfg(log, &["/setactive", &previous])?;
    log.info(format!("Restored power scheme {previous}."));

    if scheme_exists(LOW_POWER_GUID) && powercfg(log, &["/delete", LOW_POWER_GUID]).is_ok() {
        log.info("Removed the RustCleanup Low Power scheme.");
    }
    let _ = registry::delete_value(Hive::CurrentUser, STATE_KEY, PREVIOUS_VALUE);
    log.success("Low Power Mode off.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_guid;

    #[test]
    fn parses_powercfg_output() {
        let line = "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)";
        assert_eq!(
            parse_guid(line).as_deref(),
            Some("381b4222-f694-41f0-9685-ff5bb260df2e")
        );
        assert_eq!(parse_guid("no guid here"), None);
    }
}
