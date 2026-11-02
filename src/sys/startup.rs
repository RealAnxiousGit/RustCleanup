//! "Open on startup": a Task Scheduler logon task.
//!
//! The usual Run registry key can't be used: Windows silently skips programs
//! that need Administrator rights there. A logon task with "run with highest
//! privileges" starts RustCleanup elevated, without a UAC prompt.

use super::tasks::{self, current_user, xml_escape};
use crate::events::Logger;
use crate::i18n::{tr, trf};

const TASK: &str = r"\RustCleanup";
/// Passed to the exe when Task Scheduler starts it at sign-in.
pub const STARTUP_ARG: &str = "--startup";

/// The program the startup task runs, or `None` if there is no task.
pub fn registered_path() -> Option<String> {
    tasks::exec_path(TASK)
}

fn current_exe() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// `DOMAIN\user` for the signed-in account.
/// The task definition: at this user's sign-in, run this exe with highest
/// privileges, on battery too, with no time limit and one copy at a time.
fn task_xml(exe: &str) -> String {
    let user = xml_escape(&current_user());
    format!(
        r#"<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Starts RustCleanup when you sign in.</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{arg}</Arguments></Exec></Actions>
</Task>"#,
        exe = xml_escape(exe),
        arg = STARTUP_ARG,
    )
}

/// Adds or removes the startup task.
pub fn set_enabled(log: &Logger, enable: bool) -> Result<(), String> {
    let result = if enable {
        tasks::register(TASK, &task_xml(&current_exe()?))
    } else {
        tasks::delete(TASK)
    };
    result.map_err(|e| trf("Startup task not changed: {0}", &[&e]))?;
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
