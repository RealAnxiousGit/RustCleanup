//! Memory figures (physical RAM and page file) and the Clean RAM operation.
//!
//! Clean RAM trims every process working set with `EmptyWorkingSet` and can
//! flush the modified list and purge the standby list. Trimming does not
//! delete anyone's memory: pages move to the standby or modified lists and
//! are faulted back in on demand. Purging the standby list drops cached file
//! pages, which is safe but may make the next launch of a recently used
//! program slightly slower.

use std::ffi::c_void;

use windows::Win32::System::ProcessStatus::{
    EmptyWorkingSet, EnumProcesses, GetPerformanceInfo, PERFORMANCE_INFORMATION,
};
use windows::Win32::System::SystemInformation::{
    GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
};

use super::format::fmt_bytes;
use super::privilege::{OwnedHandle, enable_privilege};
use crate::events::Logger;
use crate::i18n::{tr, trf};

/// Physical RAM, as Task Manager's Memory page reports it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Physical {
    /// RAM Windows can use (installed minus hardware-reserved).
    pub usable: u64,
    /// RAM fitted in the machine, if the firmware reports it.
    pub installed: Option<u64>,
    pub available: u64,
    /// Standby + modified file cache, reclaimable on demand.
    pub cached: u64,
}

impl Physical {
    pub fn in_use(&self) -> u64 {
        self.usable.saturating_sub(self.available)
    }
    pub fn fraction(&self) -> f32 {
        self.in_use() as f32 / self.usable.max(1) as f32
    }
}

/// One page file on disk.
#[derive(Clone, Debug)]
pub struct PageFile {
    pub path: String,
    pub size: u64,
    pub in_use: u64,
    pub peak: u64,
}

/// Virtual memory: page files plus the system commit charge.
#[derive(Clone, Debug, Default)]
pub struct Virtual {
    pub page_files: Vec<PageFile>,
    /// Committed memory (RAM + page file) and the limit, like Task Manager's "Committed".
    pub committed: u64,
    pub commit_limit: u64,
}

impl Virtual {
    pub fn page_file_size(&self) -> u64 {
        self.page_files.iter().map(|p| p.size).sum()
    }
    pub fn page_file_in_use(&self) -> u64 {
        self.page_files.iter().map(|p| p.in_use).sum()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemorySnapshot {
    pub physical: Option<Physical>,
    pub virtual_mem: Option<Virtual>,
}

/// Reads all memory figures. Cheap (a few microseconds), so the UI can
/// refresh it every second.
pub fn snapshot() -> MemorySnapshot {
    let perf = performance_info();
    MemorySnapshot {
        physical: physical(perf.as_ref()),
        virtual_mem: virtual_mem(perf.as_ref()),
    }
}

fn performance_info() -> Option<PERFORMANCE_INFORMATION> {
    let mut info = PERFORMANCE_INFORMATION {
        cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is correctly sized and initialised.
    unsafe { GetPerformanceInfo(&mut info, info.cb) }.ok()?;
    Some(info)
}

fn physical(perf: Option<&PERFORMANCE_INFORMATION>) -> Option<Physical> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a correctly sized, initialised MEMORYSTATUSEX.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;

    let mut installed_kb = 0u64;
    // SAFETY: valid out-pointer.
    let installed = unsafe { GetPhysicallyInstalledSystemMemory(&mut installed_kb) }
        .ok()
        .map(|_| installed_kb * 1024)
        .filter(|&b| b >= status.ullTotalPhys);

    let cached = perf
        .map(|p| p.SystemCache as u64 * p.PageSize as u64)
        .unwrap_or(0);
    Some(Physical {
        usable: status.ullTotalPhys,
        installed,
        available: status.ullAvailPhys,
        cached,
    })
}

// NtQuerySystemInformation / NtSetSystemInformation live in ntdll and are not
// covered by the `windows` crate's Win32 bindings, so they are declared here.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: i32,
        info: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    fn NtSetSystemInformation(class: i32, info: *mut c_void, length: u32) -> i32;
}

const SYSTEM_PAGEFILE_INFORMATION_CLASS: i32 = 18;
const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
const MEMORY_FLUSH_MODIFIED_LIST: i32 = 3;
const MEMORY_PURGE_STANDBY_LIST: i32 = 4;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;

/// `SYSTEM_PAGEFILE_INFORMATION` (sizes are in pages).
#[repr(C)]
struct PageFileInfo {
    next_entry_offset: u32,
    total_size: u32,
    total_in_use: u32,
    peak_usage: u32,
    name_length: u16,
    name_max_length: u16,
    name_buffer: *const u16,
}

/// Reads every page file's size and usage, plus the commit charge.
fn virtual_mem(perf: Option<&PERFORMANCE_INFORMATION>) -> Option<Virtual> {
    let page = perf.map(|p| p.PageSize as u64).unwrap_or(4096);
    let mut buf = vec![0u64; 512]; // u64 keeps the buffer 8-byte aligned for the struct
    let mut page_files = Vec::new();
    loop {
        let bytes = (buf.len() * 8) as u32;
        let mut returned = 0u32;
        // SAFETY: the kernel writes at most `bytes` into our buffer.
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PAGEFILE_INFORMATION_CLASS,
                buf.as_mut_ptr().cast(),
                bytes,
                &mut returned,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH && buf.len() < 65_536 {
            buf.resize(buf.len() * 4, 0);
            continue;
        }
        if status < 0 {
            break;
        }
        if returned == 0 {
            break; // no page file configured
        }
        let base = buf.as_ptr() as *const u8;
        let mut offset = 0usize;
        loop {
            if offset + std::mem::size_of::<PageFileInfo>() > returned as usize {
                break;
            }
            // SAFETY: offset stays within the bytes the kernel returned, and
            // entries are 8-byte aligned by the kernel.
            let entry = unsafe { &*(base.add(offset) as *const PageFileInfo) };
            let name = if entry.name_buffer.is_null() {
                String::new()
            } else {
                // SAFETY: the kernel points the name into our buffer with the given length.
                let units = unsafe {
                    std::slice::from_raw_parts(entry.name_buffer, entry.name_length as usize / 2)
                };
                String::from_utf16_lossy(units)
            };
            page_files.push(PageFile {
                path: name.trim_start_matches(r"\??\").to_string(),
                size: entry.total_size as u64 * page,
                in_use: entry.total_in_use as u64 * page,
                peak: entry.peak_usage as u64 * page,
            });
            if entry.next_entry_offset == 0 {
                break;
            }
            offset += entry.next_entry_offset as usize;
        }
        break;
    }

    let (committed, commit_limit) = perf
        .map(|p| (p.CommitTotal as u64 * page, p.CommitLimit as u64 * page))
        .unwrap_or((0, 0));
    if commit_limit == 0 && page_files.is_empty() {
        return None;
    }
    Some(Virtual {
        page_files,
        committed,
        commit_limit,
    })
}

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
    if status >= 0 {
        Ok(())
    } else {
        Err(format!("NTSTATUS 0x{:08X}", status as u32))
    }
}

/// Trims a single process's working set. Used by both Clean RAM and the
/// task list's Optimize action.
pub fn trim_process(pid: u32) -> Result<(), String> {
    // SAFETY: OpenProcess returns an owned handle that OwnedHandle closes.
    unsafe {
        let handle = OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
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

/// What one Clean RAM pass did and how long each stage took.
#[derive(Clone, Copy, Debug, Default)]
pub struct CleanReport {
    pub trimmed: usize,
    pub skipped: usize,
    /// Trimming every working set.
    pub trim_time: std::time::Duration,
    /// Flushing the modified list and purging the standby list (zero when off).
    pub purge_time: std::time::Duration,
    /// Available RAM before and after (after the settle pause).
    pub available_before: u64,
    pub available_after: u64,
}

impl CleanReport {
    pub fn freed(&self) -> u64 {
        self.available_after.saturating_sub(self.available_before)
    }
}

/// Runs the full Clean RAM operation, logging progress as it goes.
pub fn clean_ram(log: &Logger, purge_standby: bool) -> Result<(), String> {
    let report = clean_ram_report(log, purge_standby)?;
    if report.available_before > 0 && report.available_after > 0 {
        log.success(trf(
            "Clean RAM finished. {0} freed, {1} now available.",
            &[&fmt_bytes(report.freed()), &fmt_bytes(report.available_after)],
        ));
    } else {
        log.success(tr("Clean RAM finished."));
    }
    Ok(())
}

/// Clean RAM without the final summary line, returning its figures instead.
pub fn clean_ram_report(log: &Logger, purge_standby: bool) -> Result<CleanReport, String> {
    let before = physical(None);

    // Needed to open protected/system processes and to call EmptyWorkingSet on them.
    for privilege in ["SeDebugPrivilege", "SeIncreaseQuotaPrivilege"] {
        if let Err(e) = enable_privilege(privilege) {
            log.warn(trf("Could not enable {0}: {1}", &[&privilege, &e]));
        }
    }

    let started = std::time::Instant::now();
    let mut trimmed = 0usize;
    let mut skipped = 0usize;
    for pid in all_pids()?.into_iter().filter(|&p| p > 4) {
        match trim_process(pid) {
            Ok(()) => trimmed += 1,
            // Protected processes (anti-malware, csrss, ...) refuse access; that's expected.
            Err(_) => skipped += 1,
        }
    }
    let trim_time = started.elapsed();
    log.info(trf(
        "Trimmed {0} processes. {1} protected processes were skipped.",
        &[&trimmed, &skipped],
    ));

    let started = std::time::Instant::now();
    if purge_standby {
        match enable_privilege("SeProfileSingleProcessPrivilege") {
            Err(e) => log.warn(trf("Standby list not purged: {0}", &[&e])),
            Ok(()) => {
                if let Err(e) = memory_list_command(MEMORY_FLUSH_MODIFIED_LIST) {
                    log.warn(trf("Flushing the modified page list failed: {0}", &[&e]));
                }
                match memory_list_command(MEMORY_PURGE_STANDBY_LIST) {
                    Ok(()) => log.info(tr("Standby list purged.")),
                    Err(e) => log.warn(trf("Purging the standby list failed: {0}", &[&e])),
                }
            }
        }
    }
    let purge_time = if purge_standby {
        started.elapsed()
    } else {
        std::time::Duration::ZERO
    };

    // Give the memory manager a moment to rebalance before measuring.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let after = physical(None);
    Ok(CleanReport {
        trimmed,
        skipped,
        trim_time,
        purge_time,
        available_before: before.map_or(0, |b| b.available),
        available_after: after.map_or(0, |a| a.available),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_memory_figures() {
        let snap = snapshot();
        let phys = snap.physical.expect("physical memory");
        assert!(phys.usable > 0 && phys.available <= phys.usable);
        let virt = snap.virtual_mem.expect("virtual memory");
        assert!(virt.commit_limit >= phys.usable / 2);
        for pf in &virt.page_files {
            assert!(pf.in_use <= pf.size, "{pf:?}");
            assert!(pf.path.to_ascii_lowercase().ends_with(".sys"), "{pf:?}");
        }
    }
}
