//! Clean RAM: trims every process working set with `EmptyWorkingSet` and
//! optionally flushes the modified list and purges the standby list.
//!
//! Trimming does not "delete" anyone's memory: pages move to the standby or
//! modified lists and are faulted back in on demand. Purging the standby
//! list drops cached file pages, which is safe but may make the next launch
//! of a recently used program slightly slower.

use std::ffi::c_void;

use windows::Win32::System::ProcessStatus::{EmptyWorkingSet, EnumProcesses};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
};

use super::util::{OwnedHandle, enable_privilege, fmt_bytes};
use crate::events::Logger;

/// Snapshot of physical memory usage.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemStatus {
    pub total: u64,
    pub available: u64,
    pub load_percent: u32,
}

impl MemStatus {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

/// Reads current physical memory usage. Cheap enough to call every frame.
pub fn mem_status() -> Option<MemStatus> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a correctly sized, initialised MEMORYSTATUSEX.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(MemStatus {
        total: status.ullTotalPhys,
        available: status.ullAvailPhys,
        load_percent: status.dwMemoryLoad,
    })
}

// NtSetSystemInformation is exported by ntdll but not covered by the
// `windows` crate's Win32 bindings, so we declare it ourselves.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetSystemInformation(class: i32, info: *mut c_void, length: u32) -> i32;
}

const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
const MEMORY_FLUSH_MODIFIED_LIST: i32 = 3;
const MEMORY_PURGE_STANDBY_LIST: i32 = 4;

/// Sends one memory-list command (flush modified / purge standby) to the kernel.
fn memory_list_command(command: i32) -> Result<(), String> {
    let mut cmd = command;
    // SAFETY: the kernel reads a single i32 from a valid pointer.
    let status = unsafe {
        NtSetSystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            &mut cmd as *mut i32 as *mut c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    if status >= 0 { Ok(()) } else { Err(format!("NTSTATUS 0x{:08X}", status as u32)) }
}

/// Trims a single process's working set. Used by both Clean RAM and the
/// task optimizer.
pub fn trim_process(pid: u32) -> Result<(), String> {
    // SAFETY: OpenProcess returns an owned handle that OwnedHandle closes.
    unsafe {
        let handle = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| e.message().to_string())?;
        let handle = OwnedHandle(handle);
        EmptyWorkingSet(handle.0).map_err(|e| e.message().to_string())
    }
}

/// Lists the PIDs of all running processes.
fn all_pids() -> Result<Vec<u32>, String> {
    let mut pids = vec![0u32; 2048];
    loop {
        let mut needed = 0u32;
        let bytes = (pids.len() * std::mem::size_of::<u32>()) as u32;
        // SAFETY: the buffer is `bytes` long and `needed` is a valid out-param.
        unsafe { EnumProcesses(pids.as_mut_ptr(), bytes, &mut needed) }
            .map_err(|e| format!("EnumProcesses: {e}"))?;
        if needed < bytes {
            pids.truncate(needed as usize / std::mem::size_of::<u32>());
            return Ok(pids);
        }
        // Buffer was full, so there may be more processes: grow and retry.
        pids.resize(pids.len() * 2, 0);
    }
}

/// Runs the full Clean RAM operation, logging progress as it goes.
pub fn clean_ram(log: &Logger, purge_standby: bool) -> Result<(), String> {
    let before = mem_status();

    // Needed to open protected/system processes and to call EmptyWorkingSet on them.
    for privilege in ["SeDebugPrivilege", "SeIncreaseQuotaPrivilege"] {
        if let Err(e) = enable_privilege(privilege) {
            log.warn(format!("Could not enable {privilege}: {e}"));
        }
    }

    let pids = all_pids()?;
    let mut trimmed = 0usize;
    let mut skipped = 0usize;
    for pid in pids.into_iter().filter(|&p| p > 4) {
        match trim_process(pid) {
            Ok(()) => trimmed += 1,
            // Protected processes (anti-malware, csrss, ...) refuse access; that's expected.
            Err(_) => skipped += 1,
        }
    }
    log.info(format!("Trimmed working sets of {trimmed} processes ({skipped} protected processes skipped)."));

    if purge_standby {
        match enable_privilege("SeProfileSingleProcessPrivilege") {
            Err(e) => log.warn(format!("Standby list not purged: {e}")),
            Ok(()) => {
                if let Err(e) = memory_list_command(MEMORY_FLUSH_MODIFIED_LIST) {
                    log.warn(format!("Flushing modified page list failed: {e}"));
                }
                match memory_list_command(MEMORY_PURGE_STANDBY_LIST) {
                    Ok(()) => log.info("Standby list purged."),
                    Err(e) => log.warn(format!("Purging standby list failed: {e}")),
                }
            }
        }
    }

    // Give the memory manager a moment to rebalance before measuring.
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let (Some(b), Some(a)) = (before, mem_status()) {
        let freed = a.available.saturating_sub(b.available);
        log.success(format!(
            "Clean RAM finished: {} available now (+{} freed, load {}% -> {}%).",
            fmt_bytes(a.available),
            fmt_bytes(freed),
            b.load_percent,
            a.load_percent
        ));
    } else {
        log.success("Clean RAM finished.");
    }
    Ok(())
}
