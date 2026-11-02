//! "Open on startup": a Task Scheduler logon task.
//!
//! The usual Run registry key can't be used: Windows silently skips programs
//! that need Administrator rights there. A logon task with "run with highest
//! privileges" starts RustCleanup elevated, without a UAC prompt.

use super::command::run_powershell;
use crate::events::Logger;
use crate::i18n::{tr, trf};

const TASK: &str = "RustCleanup";
/// Passed to the exe when Task Scheduler starts it at sign-in.
pub const STARTUP_ARG: &str = "--startup";

/// Quotes a value for a single-quoted PowerShell string.
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The program the startup task runs, or `None` if there is no task.
pub fn registered_path() -> Option<String> {
    let script = format!(
        "$t = Get-ScheduledTask -TaskPath '\\' -TaskName {} -ErrorAction SilentlyContinue; \
         if ($t) {{ 'PATH|' + $t.Actions[0].Execute }}",
        ps_quote(TASK)
    );
    let out = run_powershell(&script).ok()?;
    out.stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("PATH|").map(str::to_string))
}

fn current_exe() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// Adds or removes the startup task.
pub fn set_enabled(log: &Logger, enable: bool) -> Result<(), String> {
    let script = if enable {
        format!(
            r#"$me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$a = New-ScheduledTaskAction -Execute {exe} -Argument {arg}
$t = New-ScheduledTaskTrigger -AtLogOn -User $me
$p = New-ScheduledTaskPrincipal -UserId $me -LogonType Interactive -RunLevel Highest
$s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -Priority 4
Register-ScheduledTask -TaskPath '\' -TaskName {task} -Action $a -Trigger $t -Principal $p -Settings $s -Description 'Starts RustCleanup when you sign in.' -Force -ErrorAction Stop | Out-Null
'OK'"#,
            exe = ps_quote(&current_exe()?),
            arg = ps_quote(STARTUP_ARG),
            task = ps_quote(TASK),
        )
    } else {
        format!(
            "Unregister-ScheduledTask -TaskPath '\\' -TaskName {} -Confirm:$false -ErrorAction SilentlyContinue; 'OK'",
            ps_quote(TASK)
        )
    };
    let out = run_powershell(&script)?;
    if !out.stdout.contains("OK") {
        return Err(trf("Startup task not changed: {0}", &[&out.summary()]));
    }
    log.success(if enable {
        tr("RustCleanup will open when you sign in to Windows.")
    } else {
        tr("RustCleanup will no longer open when you sign in.")
    });
    Ok(())
}

/// Makes the task match the saved choice. Run on launch, so the task follows
/// the exe if it was moved or replaced by an update.
pub fn sync(log: &Logger, wanted: bool) -> Result<(), String> {
    let registered = registered_path();
    let here = current_exe()?;
    match (wanted, registered) {
        (true, Some(path)) if path.eq_ignore_ascii_case(&here) => Ok(()),
        (true, _) => set_enabled(log, true),
        (false, Some(_)) => set_enabled(log, false),
        (false, None) => Ok(()),
    }
}
