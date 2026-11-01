//! Optimize Task: process listing (via `sysinfo`) and per-process actions —
//! trim memory, lower priority, suspend/resume and end task.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME,
    PROCESS_TERMINATE, SetPriorityClass, TerminateProcess,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

use super::util::OwnedHandle;

/// Processes that keep Windows alive. Suspending or killing any of these can
/// freeze or blue-screen the machine, so the optimizer refuses to.
const CRITICAL: [&str; 19] = [
    "system", "idle", "registry", "secure system", "memory compression", "smss.exe", "csrss.exe",
    "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "lsaiso.exe", "svchost.exe",
    "dwm.exe", "fontdrvhost.exe", "sihost.exe", "explorer.exe", "msmpeng.exe", "audiodg.exe",
];

/// Whether a process owns a visible top-level window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcKind {
    App,
    Background,
}

/// One row of the process list.
#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    /// CPU usage as a share of the whole machine (0–100), like Task Manager.
    pub cpu: f32,
    pub memory: u64,
    pub kind: ProcKind,
    /// System-critical or RustCleanup itself: only memory trimming is allowed.
    pub protected: bool,
}

/// True if a process with this exact image name is running.
pub fn is_running(name: &str) -> bool {
    let Ok(mut sys) = System::new() else { return false };
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes_by_exact_name(OsStr::new(name)).next().is_some()
}

/// PIDs that own at least one visible, unowned, titled top-level window —
/// i.e. what Task Manager lists under "Apps".
fn windowed_pids() -> HashSet<u32> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the &mut HashSet passed to EnumWindows below and
        // outlives the enumeration.
        unsafe {
            let set = &mut *(lparam.0 as *mut HashSet<u32>);
            let unowned = GetWindow(hwnd, GW_OWNER).map(|o| o.is_invalid()).unwrap_or(true);
            if IsWindowVisible(hwnd).as_bool() && unowned && GetWindowTextLengthW(hwnd) > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != 0 {
                    set.insert(pid);
                }
            }
        }
        BOOL(1) // keep enumerating
    }

    let mut set = HashSet::new();
    // SAFETY: the callback only touches `set` during this synchronous call.
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut set as *mut _ as isize));
    }
    set
}

/// Background sampler that refreshes the process list while the Optimize
/// Task window is open. CPU usage needs two samples, so this runs on its own
/// thread instead of blocking the UI.
pub struct ProcessMonitor {
    pub snapshot: Arc<Mutex<Vec<ProcInfo>>>,
    stop: Arc<AtomicBool>,
}

impl ProcessMonitor {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        let snapshot = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (snap, stop_flag) = (snapshot.clone(), stop.clone());

        std::thread::spawn(move || {
            let Ok(mut sys) = System::new() else { return };
            let refresh = ProcessRefreshKind::nothing().with_cpu().with_memory();
            let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
            let me = std::process::id();
            let mut first = true;

            while !stop_flag.load(Ordering::Relaxed) {
                sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
                let apps = windowed_pids();
                let mut list: Vec<ProcInfo> = sys
                    .processes()
                    .values()
                    .filter(|p| p.pid().as_u32() != 0)
                    .map(|p| {
                        let pid = p.pid().as_u32();
                        let name = p.name().to_string_lossy().into_owned();
                        let lower = name.to_ascii_lowercase();
                        ProcInfo {
                            pid,
                            cpu: if first { 0.0 } else { (p.cpu_usage() / cores).min(100.0) },
                            memory: p.memory(),
                            kind: if apps.contains(&pid) { ProcKind::App } else { ProcKind::Background },
                            protected: pid == me || pid <= 4 || CRITICAL.contains(&lower.as_str()),
                            name,
                        }
                    })
                    .collect();
                list.sort_by_key(|p| std::cmp::Reverse(p.memory));
                if let Ok(mut guard) = snap.lock() {
                    *guard = list;
                }
                repaint();
                // First pass only establishes the CPU baseline; sample again quickly.
                let wait = if first { 400 } else { 1500 };
                first = false;
                for _ in 0..wait / 100 {
                    if stop_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });

        Self { snapshot, stop }
    }
}

impl Drop for ProcessMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// NtSuspendProcess / NtResumeProcess suspend every thread of a process at
// once; they're the same calls Resource Monitor uses.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(handle: HANDLE) -> i32;
    fn NtResumeProcess(handle: HANDLE) -> i32;
}

fn open(pid: u32, access: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS) -> Result<OwnedHandle, String> {
    // SAFETY: the returned handle is owned and closed by OwnedHandle.
    unsafe { OpenProcess(access, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| e.message().to_string())
}

/// "Optimize": trims the working set and drops priority to Below Normal so
/// the process yields CPU to whatever you're actively using.
pub fn optimize(pid: u32) -> Result<(), String> {
    super::ram::trim_process(pid)?;
    let handle = open(pid, PROCESS_SET_INFORMATION)?;
    // SAFETY: valid handle with PROCESS_SET_INFORMATION access.
    unsafe { SetPriorityClass(handle.0, BELOW_NORMAL_PRIORITY_CLASS) }.map_err(|e| e.message().to_string())
}

pub fn suspend(pid: u32) -> Result<(), String> {
    let handle = open(pid, PROCESS_SUSPEND_RESUME)?;
    // SAFETY: valid handle with PROCESS_SUSPEND_RESUME access.
    let status = unsafe { NtSuspendProcess(handle.0) };
    if status >= 0 { Ok(()) } else { Err(format!("NTSTATUS 0x{:08X}", status as u32)) }
}

pub fn resume(pid: u32) -> Result<(), String> {
    let handle = open(pid, PROCESS_SUSPEND_RESUME)?;
    // SAFETY: valid handle with PROCESS_SUSPEND_RESUME access.
    let status = unsafe { NtResumeProcess(handle.0) };
    if status >= 0 { Ok(()) } else { Err(format!("NTSTATUS 0x{:08X}", status as u32)) }
}

/// Ends a process immediately (like Task Manager's "End task").
pub fn terminate(pid: u32) -> Result<(), String> {
    // Prefer sysinfo's kill (it handles PID reuse checks); fall back to Win32.
    if let Ok(mut sys) = System::new() {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, ProcessRefreshKind::nothing());
        if let Some(p) = sys.process(Pid::from_u32(pid))
            && p.kill() {
                return Ok(());
            }
    }
    let handle = open(pid, PROCESS_TERMINATE)?;
    // SAFETY: valid handle with PROCESS_TERMINATE access.
    unsafe { TerminateProcess(handle.0, 1) }.map_err(|e| e.message().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_lists_processes() {
        let monitor = ProcessMonitor::start(|| {});
        std::thread::sleep(Duration::from_millis(1500));
        let snap = monitor.snapshot.lock().unwrap().clone();
        assert!(snap.len() > 10);
        let me = snap.iter().find(|p| p.pid == std::process::id()).expect("own process listed");
        assert!(me.protected);
        assert!(snap.iter().any(|p| p.name.eq_ignore_ascii_case("csrss.exe") && p.protected));
        assert!(is_running("explorer.exe"));
    }
}
