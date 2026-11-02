//! Disable Copilot: the TurnOffWindowsCopilot policy plus the taskbar button.

use super::registry::{self, Hive};
use crate::events::Logger;
use crate::i18n::tr;

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
pub fn set_copilot_disabled(
    log: &Logger,
    disable: bool,
    restart_shell: bool,
) -> Result<(), String> {
    if disable {
        // Policy for both the current user and the whole machine.
        registry::set_dword(
            Hive::CurrentUser,
            COPILOT_POLICY,
            "TurnOffWindowsCopilot",
            1,
        )?;
        registry::set_dword(
            Hive::LocalMachine,
            COPILOT_POLICY,
            "TurnOffWindowsCopilot",
            1,
        )?;
        // Hide the taskbar button on builds that still have one.
        registry::set_dword(Hive::CurrentUser, EXPLORER_ADVANCED, "ShowCopilotButton", 0)?;
        log.info(tr(
            "Copilot policy TurnOffWindowsCopilot=1 written to HKCU and HKLM.",
        ));
    } else {
        registry::delete_value(Hive::CurrentUser, COPILOT_POLICY, "TurnOffWindowsCopilot")?;
        registry::delete_value(Hive::LocalMachine, COPILOT_POLICY, "TurnOffWindowsCopilot")?;
        registry::delete_value(Hive::CurrentUser, EXPLORER_ADVANCED, "ShowCopilotButton")?;
        log.info(tr("Copilot policy removed; Windows defaults restored."));
    }
    if restart_shell {
        super::shell::restart_explorer(log)?;
    }
    log.success(if disable {
        tr("Copilot disabled.")
    } else {
        tr("Copilot re-enabled.")
    });
    Ok(())
}
