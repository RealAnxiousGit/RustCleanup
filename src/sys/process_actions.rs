//! Actions on a single process: optimize, suspend, resume and end.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_SET_INFORMATION,
    PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, SetPriorityClass, TerminateProcess,
};

use super::privilege::OwnedHandle;

// NtSuspendProcess / NtResumeProcess suspend every thread of a process at
// once; they are the same calls Resource Monitor uses.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(handle: HANDLE) -> i32;
    fn NtResumeProcess(handle: HANDLE) -> i32;
}

fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<OwnedHandle, String> {
    // SAFETY: the returned handle is owned and closed by OwnedHandle.
    unsafe { OpenProcess(access, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| e.message().to_string())
}

fn nt_result(status: i32) -> Result<(), String> {
    if status >= 0 {
        Ok(())
    } else {
        Err(format!("NTSTATUS 0x{:08X}", status as u32))
    }
}

/// "Optimize": trims the working set and drops priority to Below Normal so
/// the process yields CPU to whatever you're actively using.
pub fn optimize(pid: u32) -> Result<(), String> {
    super::memory::trim_process(pid)?;
    let handle = open(pid, PROCESS_SET_INFORMATION)?;
    // SAFETY: valid handle with PROCESS_SET_INFORMATION access.
    unsafe { SetPriorityClass(handle.0, BELOW_NORMAL_PRIORITY_CLASS) }
        .map_err(|e| e.message().to_string())
}

pub fn suspend(pid: u32) -> Result<(), String> {
    let handle = open(pid, PROCESS_SUSPEND_RESUME)?;
    // SAFETY: valid handle with PROCESS_SUSPEND_RESUME access.
    nt_result(unsafe { NtSuspendProcess(handle.0) })
}

pub fn resume(pid: u32) -> Result<(), String> {
    let handle = open(pid, PROCESS_SUSPEND_RESUME)?;
    // SAFETY: valid handle with PROCESS_SUSPEND_RESUME access.
    nt_result(unsafe { NtResumeProcess(handle.0) })
}

/// Ends a process immediately (like Task Manager's "End task").
pub fn terminate(pid: u32) -> Result<(), String> {
    // Prefer sysinfo's kill (it guards against PID reuse); fall back to Win32.
    if let Ok(mut sys) = System::new() {
        let target = Pid::from_u32(pid);
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if sys.process(target).is_some_and(|p| p.kill()) {
            return Ok(());
        }
    }
    let handle = open(pid, PROCESS_TERMINATE)?;
    // SAFETY: valid handle with PROCESS_TERMINATE access.
    unsafe { TerminateProcess(handle.0, 1) }.map_err(|e| e.message().to_string())
}
