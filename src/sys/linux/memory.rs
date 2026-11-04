//! Memory figures (RAM and swap) and the Clean RAM operation on Linux.
//!
//! Clean RAM is the Linux take on what Windows does with `EmptyWorkingSet`
//! and the standby-list purge:
//!
//! 1. `sync`, so dirty file pages are written out and become reclaimable.
//! 2. Every process gets `process_madvise(MADV_PAGEOUT)` over its mappings.
//!    Like trimming a working set, nothing is lost: pages go to swap or back
//!    to their file and fault back in on demand.
//! 3. Without root (or on kernels without `process_madvise`), the session's
//!    own cgroup v2 `memory.reclaim` does the same job for the user's apps.
//! 4. With "purge the file cache" on, `drop_caches` drops clean page cache,
//!    dentries and inodes, and `compact_memory` defragments free memory.

use std::fs;

use crate::sys::format::fmt_bytes;
use super::privilege::is_elevated;
use crate::events::Logger;
use crate::i18n::{tr, trf};

/// Physical RAM.
#[derive(Clone, Copy, Debug, Default)]
pub struct Physical {
    /// RAM the kernel can use (`MemTotal`).
    pub usable: u64,
    /// RAM fitted in the machine. Linux only reports it to root, so `None`.
    pub installed: Option<u64>,
    pub available: u64,
    /// Page cache and reclaimable slab, freed on demand.
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

/// One swap file or partition.
#[derive(Clone, Debug)]
pub struct PageFile {
    pub path: String,
    pub size: u64,
    pub in_use: u64,
    pub peak: u64,
}

/// Virtual memory: swap plus the kernel's commit charge.
#[derive(Clone, Debug, Default)]
pub struct Virtual {
    pub page_files: Vec<PageFile>,
    /// `Committed_AS`, and RAM plus swap as the limit.
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

/// `/proc/meminfo` as (name, bytes) pairs.
fn meminfo() -> Option<std::collections::HashMap<String, u64>> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    Some(parse_meminfo(&text))
}

fn parse_meminfo(text: &str) -> std::collections::HashMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let mut parts = rest.split_whitespace();
            let value: u64 = parts.next()?.parse().ok()?;
            let scale = if parts.next() == Some("kB") { 1024 } else { 1 };
            Some((name.trim().to_string(), value * scale))
        })
        .collect()
}

fn physical() -> Option<Physical> {
    let m = meminfo()?;
    let usable = *m.get("MemTotal")?;
    let available = m
        .get("MemAvailable")
        .copied()
        .unwrap_or_else(|| m.get("MemFree").copied().unwrap_or(0));
    let cached = ["Cached", "Buffers", "SReclaimable"]
        .iter()
        .filter_map(|k| m.get(*k))
        .sum::<u64>()
        .saturating_sub(m.get("Shmem").copied().unwrap_or(0));
    Some(Physical {
        usable,
        installed: None,
        available: available.min(usable),
        cached,
    })
}

fn swaps() -> Vec<PageFile> {
    let text = fs::read_to_string("/proc/swaps").unwrap_or_default();
    parse_swaps(&text)
}

fn parse_swaps(text: &str) -> Vec<PageFile> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            let size = cols.get(2)?.parse::<u64>().ok()? * 1024;
            let used = cols.get(3)?.parse::<u64>().ok()? * 1024;
            Some(PageFile {
                path: cols[0].replace("\\040", " "),
                size,
                in_use: used.min(size),
                peak: used.min(size),
            })
        })
        .collect()
}

fn virtual_mem() -> Option<Virtual> {
    let m = meminfo()?;
    Some(Virtual {
        page_files: swaps(),
        committed: *m.get("Committed_AS")?,
        commit_limit: m.get("MemTotal")? + m.get("SwapTotal").copied().unwrap_or(0),
    })
}

/// Reads all memory figures. Cheap, so the UI can refresh it every second.
pub fn snapshot() -> MemorySnapshot {
    MemorySnapshot {
        physical: physical(),
        virtual_mem: virtual_mem(),
    }
}

const MADV_PAGEOUT: libc::c_int = 21;
/// `UIO_MAXIOV`: the most ranges one `process_madvise` call takes.
const MAX_RANGES: usize = 1024;

/// Address ranges worth paging out: everything except the kernel's
/// special mappings.
fn ranges(pid: u32) -> Result<Vec<(usize, usize)>, String> {
    let maps = fs::read_to_string(format!("/proc/{pid}/maps")).map_err(|e| e.to_string())?;
    Ok(maps
        .lines()
        .filter(|l| !l.ends_with("[vsyscall]") && !l.ends_with("[vvar]") && !l.ends_with("[vdso]"))
        .filter_map(|l| {
            let (start, end) = l.split_whitespace().next()?.split_once('-')?;
            let start = usize::from_str_radix(start, 16).ok()?;
            let end = usize::from_str_radix(end, 16).ok()?;
            (end > start).then_some((start, end - start))
        })
        .collect())
}

fn advise(pidfd: libc::c_int, ranges: &[(usize, usize)]) -> Result<(), i32> {
    let iov: Vec<libc::iovec> = ranges
        .iter()
        .map(|&(base, len)| libc::iovec {
            iov_base: base as *mut libc::c_void,
            iov_len: len,
        })
        .collect();
    // SAFETY: `iov` is a valid array; the kernel only reads it and never
    // dereferences the ranges in our own address space.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_process_madvise,
            pidfd,
            iov.as_ptr(),
            iov.len(),
            MADV_PAGEOUT,
            0,
        )
    };
    if rc < 0 {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    } else {
        Ok(())
    }
}

/// Pages a single process out to swap or its files. Used by both Clean RAM
/// and the task list's Optimize action. Needs root for other users'
/// processes.
pub fn trim_process(pid: u32) -> Result<(), String> {
    let ranges = ranges(pid)?;
    // SAFETY: plain syscall; the returned descriptor is closed below.
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) } as i32;
    if pidfd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut advised = 0usize;
    let mut last_err = 0;
    for batch in ranges.chunks(MAX_RANGES) {
        match advise(pidfd, batch) {
            Ok(()) => advised += batch.len(),
            Err(e @ (libc::ENOSYS | libc::EPERM | libc::ESRCH)) => {
                last_err = e;
                break;
            }
            // One odd mapping (device memory, locked pages) fails the whole
            // batch, so go one range at a time.
            Err(e) => {
                last_err = e;
                advised += batch.iter().filter(|r| advise(pidfd, &[**r]).is_ok()).count();
            }
        }
    }
    // SAFETY: we own the descriptor.
    unsafe { libc::close(pidfd) };
    if advised > 0 || ranges.is_empty() {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(last_err).to_string())
    }
}

/// Lists the PIDs of all running user-space processes.
fn all_pids() -> Result<Vec<u32>, String> {
    let dir = fs::read_dir("/proc").map_err(|e| format!("/proc: {e}"))?;
    Ok(dir
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        // Kernel threads have no memory map to page out.
        .filter(|&pid| {
            pid > 1
                && pid != std::process::id()
                && fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| !c.is_empty())
        })
        .collect())
}

/// The cgroup v2 folder of this session, where an unprivileged user may
/// write `memory.reclaim`.
fn own_cgroup() -> Option<std::path::PathBuf> {
    let text = fs::read_to_string("/proc/self/cgroup").ok()?;
    let rel = text.lines().find_map(|l| l.strip_prefix("0::"))?;
    let mut dir = std::path::PathBuf::from("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
    // Climb to the user's session or app slice, which covers their apps.
    while let Some(parent) = dir.parent() {
        let name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !parent.join("memory.reclaim").exists() || name.starts_with("user-") || name == "user.slice" {
            break;
        }
        dir = parent.to_path_buf();
    }
    dir.join("memory.reclaim").exists().then_some(dir)
}

/// Asks the kernel to reclaim memory from a cgroup, as much as it holds.
fn reclaim_cgroup(dir: &std::path::Path) -> Result<(), String> {
    let current: u64 = fs::read_to_string(dir.join("memory.current"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if current == 0 {
        return Ok(());
    }
    // The kernel stops early (EAGAIN) once it cannot reclaim more, which is fine.
    match fs::write(dir.join("memory.reclaim"), current.to_string()) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// Runs the full Clean RAM operation, logging progress as it goes.
pub fn clean_ram(log: &Logger, purge_standby: bool) -> Result<(), String> {
    let before = physical();
    let root = is_elevated();

    // SAFETY: no arguments; flushes dirty pages so they can be reclaimed.
    unsafe { libc::sync() };

    let mut trimmed = 0usize;
    let mut skipped = 0usize;
    for pid in all_pids()? {
        match trim_process(pid) {
            Ok(()) => trimmed += 1,
            Err(_) => skipped += 1,
        }
    }
    log.info(trf(
        "Trimmed {0} processes. {1} protected processes were skipped.",
        &[&trimmed, &skipped],
    ));

    if !root || trimmed == 0 {
        if let Some(dir) = own_cgroup() {
            match reclaim_cgroup(&dir) {
                Ok(()) => log.info(tr("Asked the kernel to reclaim memory from your session.")),
                Err(e) => log.warn(trf("Reclaiming memory from your session failed: {0}", &[&e])),
            }
        }
    }

    if purge_standby {
        if root {
            match fs::write("/proc/sys/vm/drop_caches", "3") {
                Ok(()) => log.info(tr("File cache dropped.")),
                Err(e) => log.warn(trf("Dropping the file cache failed: {0}", &[&e])),
            }
            let _ = fs::write("/proc/sys/vm/compact_memory", "1");
        } else {
            log.warn(tr("The file cache can only be dropped as root."));
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    match (before, physical()) {
        (Some(b), Some(a)) => log.success(trf(
            "Clean RAM finished. {0} freed, {1} now available.",
            &[
                &fmt_bytes(a.available.saturating_sub(b.available)),
                &fmt_bytes(a.available),
            ],
        )),
        _ => log.success(tr("Clean RAM finished.")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_files() {
        let m = parse_meminfo("MemTotal:       16000 kB\nMemAvailable:    8000 kB\nHugePages_Total:       0\n");
        assert_eq!(m["MemTotal"], 16000 * 1024);
        assert_eq!(m["HugePages_Total"], 0);
        let s = parse_swaps(
            "Filename\tType\tSize\tUsed\tPriority\n/swap\\040file file 2048 1024 -2\n",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].path, "/swap file");
        assert_eq!(s[0].in_use, 1024 * 1024);
    }

    #[test]
    fn reads_memory_figures() {
        let snap = snapshot();
        let phys = snap.physical.expect("physical memory");
        assert!(phys.usable > 0 && phys.available <= phys.usable);
        assert!(snap.virtual_mem.is_some());
    }
}
