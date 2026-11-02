//! Task Scheduler through its own COM API (`ITaskService`).
//!
//! 2.0.1 drove Task Scheduler through hidden PowerShell runs. Security tools
//! treat hidden script hosts that create logon tasks as malware behaviour, so
//! the tasks are now changed directly, the same way the Task Scheduler window does.

use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::TaskScheduler::{
    IExecAction, ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN,
    TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HRESULT, Interface};

use super::com::{self, Apartment};

/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)` and `ERROR_PATH_NOT_FOUND`:
/// the task, or the folder it would be in, does not exist.
const NOT_FOUND: [HRESULT; 2] = [
    HRESULT(0x8007_0002_u32 as i32),
    HRESULT(0x8007_0003_u32 as i32),
];

fn is_missing(e: &windows::core::Error) -> bool {
    NOT_FOUND.contains(&e.code())
}

/// Splits `\Folder\Name` into the folder path and the task name.
fn split(path: &str) -> (&str, &str) {
    match path.rfind('\\') {
        Some(0) | None => ("\\", path.trim_start_matches('\\')),
        Some(i) => (&path[..i], &path[i + 1..]),
    }
}

/// A connected Task Scheduler and the COM apartment it lives in.
struct Scheduler {
    service: ITaskService,
    _com: Apartment,
}

impl Scheduler {
    fn connect() -> Result<Self, String> {
        let com = Apartment::enter()?;
        // SAFETY: plain COM calls on an initialized apartment.
        let service: ITaskService = unsafe {
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| com::message(&e))?;
            let none = VARIANT::default();
            service
                .Connect(&none, &none, &none, &none)
                .map_err(|e| com::message(&e))?;
            service
        };
        Ok(Self { service, _com: com })
    }

    fn folder(&self, path: &str) -> windows::core::Result<ITaskFolder> {
        // SAFETY: plain COM call.
        unsafe { self.service.GetFolder(&BSTR::from(path)) }
    }
}

/// The program the task's first action runs, or `None` if there is no task.
pub fn exec_path(path: &str) -> Option<String> {
    let scheduler = Scheduler::connect().ok()?;
    let (folder, name) = split(path);
    // SAFETY: plain COM calls; every result is checked.
    unsafe {
        let task = scheduler
            .folder(folder)
            .ok()?
            .GetTask(&BSTR::from(name))
            .ok()?;
        let action = task.Definition().ok()?.Actions().ok()?.get_Item(1).ok()?;
        let exec: IExecAction = action.cast().ok()?;
        let mut program = BSTR::new();
        exec.Path(&mut program).ok()?;
        Some(program.to_string())
    }
}

/// Creates the task, or replaces it if it exists, from Task Scheduler XML.
pub fn register(path: &str, xml: &str) -> Result<(), String> {
    let scheduler = Scheduler::connect()?;
    let (folder, name) = split(path);
    let none = VARIANT::default();
    // SAFETY: plain COM calls.
    unsafe {
        scheduler
            .folder(folder)
            .and_then(|f| {
                f.RegisterTask(
                    &BSTR::from(name),
                    &BSTR::from(xml),
                    TASK_CREATE_OR_UPDATE.0,
                    &none,
                    &none,
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    &none,
                )
            })
            .map(|_| ())
            .map_err(|e| com::message(&e))
    }
}

/// Deletes the task. A task that does not exist counts as deleted.
pub fn delete(path: &str) -> Result<(), String> {
    let scheduler = Scheduler::connect()?;
    let (folder, name) = split(path);
    // SAFETY: plain COM calls.
    match unsafe {
        scheduler
            .folder(folder)
            .and_then(|f| f.DeleteTask(&BSTR::from(name), 0))
    } {
        Err(e) if !is_missing(&e) => Err(com::message(&e)),
        _ => Ok(()),
    }
}

/// What happened to one task in [`set_enabled_all`].
pub enum Outcome {
    Changed,
    Missing,
    Failed(String),
}

/// Enables or disables each task by full path. `Err` means Task Scheduler
/// itself could not be reached; problems with single tasks are in the list.
pub fn set_enabled_all<'a>(
    paths: &[&'a str],
    enabled: bool,
) -> Result<Vec<(&'a str, Outcome)>, String> {
    let scheduler = Scheduler::connect()?;
    let flag = if enabled { VARIANT_TRUE } else { VARIANT_FALSE };
    Ok(paths
        .iter()
        .map(|&path| {
            let (folder, name) = split(path);
            // SAFETY: plain COM calls.
            let result = unsafe {
                scheduler
                    .folder(folder)
                    .and_then(|f| f.GetTask(&BSTR::from(name)))
                    .and_then(|t| t.SetEnabled(flag))
            };
            let outcome = match result {
                Ok(()) => Outcome::Changed,
                Err(e) if is_missing(&e) => Outcome::Missing,
                Err(e) => Outcome::Failed(com::message(&e)),
            };
            (path, outcome)
        })
        .collect())
}

/// Escapes text for an XML element.
pub fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_task_paths() {
        assert_eq!(split(r"\RustCleanup"), ("\\", "RustCleanup"));
        assert_eq!(split("RustCleanup"), ("\\", "RustCleanup"));
        assert_eq!(
            split(r"\Microsoft\Windows\Autochk\Proxy"),
            (r"\Microsoft\Windows\Autochk", "Proxy")
        );
    }

    #[test]
    fn missing_task_reads_as_none() {
        assert!(exec_path(r"\RustCleanup test task that does not exist").is_none());
    }

    #[test]
    fn escapes_xml() {
        assert_eq!(xml_escape(r"C:\A&B <x>"), r"C:\A&amp;B &lt;x&gt;");
    }
}
