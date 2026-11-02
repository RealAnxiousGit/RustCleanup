//! Background sampling of running processes for the task list.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible,
};
use windows::core::BOOL;

use crate::i18n::trf;

/// Processes that keep Windows alive. Suspending or killing any of these can
/// freeze or crash the machine, so the task list refuses to.
const CRITICAL: [&str; 19] = [
    "system",
    "idle",
    "registry",
    "secure system",
    "memory compression",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "svchost.exe",
    "dwm.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "explorer.exe",
    "msmpeng.exe",
    "audiodg.exe",
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
    /// Lower-cased name, precomputed so filtering doesn't allocate per frame.
    pub name_lower: String,
    /// CPU usage as a share of the whole machine (0 to 100), like Task Manager.
    pub cpu: f32,
    pub memory: u64,
    pub kind: ProcKind,
    /// System-critical or RustCleanup itself: only memory trimming is allowed.
    pub protected: bool,
}

/// A sample of all processes. `generation` increases with every sample so
/// the UI can tell when its cached sort/filter is stale.
#[derive(Clone, Default)]
pub struct ProcSnapshot {
    pub generation: u64,
    pub rows: Arc<Vec<ProcInfo>>,
    pub error: Option<String>,
}

/// True if a process with this exact image name is running.
pub fn is_running(name: &str) -> bool {
    let Ok(mut sys) = System::new() else {
        return false;
    };
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes_by_exact_name(OsStr::new(name))
        .next()
        .is_some()
}

/// PIDs that own at least one visible, unowned, titled top-level window,
/// i.e. what Task Manager lists under "Apps".
fn windowed_pids() -> HashSet<u32> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the &mut HashSet passed to EnumWindows below and
        // outlives the enumeration.
        unsafe {
            let set = &mut *(lparam.0 as *mut HashSet<u32>);
            let unowned = GetWindow(hwnd, GW_OWNER)
                .map(|o| o.is_invalid())
                .unwrap_or(true);
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

/// Samples the process list on its own thread while the task list is open.
/// CPU usage needs two samples, so this never blocks the UI. Dropping the
/// monitor stops the thread.
pub struct ProcessMonitor {
    snapshot: Arc<Mutex<ProcSnapshot>>,
    stop: Arc<AtomicBool>,
}

impl ProcessMonitor {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        let snapshot = Arc::new(Mutex::new(ProcSnapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stop_flag) = (snapshot.clone(), stop.clone());

        let spawned = std::thread::Builder::new()
            .name("process-monitor".into())
            .spawn(move || {
                let mut sys = match System::new() {
                    Ok(s) => s,
                    Err(e) => {
                        if let Ok(mut g) = shared.lock() {
                            g.error = Some(trf("Could not read the process list: {0}", &[&e]));
                        }
                        repaint();
                        return;
                    }
                };
                let refresh = ProcessRefreshKind::nothing().with_cpu().with_memory();
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1) as f32;
                let me = std::process::id();
                let mut generation = 0u64;

                while !stop_flag.load(Ordering::Relaxed) {
                    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
                    let apps = windowed_pids();
                    let rows: Vec<ProcInfo> = sys
                        .processes()
                        .values()
                        .filter(|p| p.pid().as_u32() != 0)
                        .map(|p| {
                            let pid = p.pid().as_u32();
                            let name = p.name().to_string_lossy().into_owned();
                            let name_lower = name.to_lowercase();
                            ProcInfo {
                                pid,
                                // The first pass only sets the CPU baseline.
                                cpu: if generation == 0 {
                                    0.0
                                } else {
                                    (p.cpu_usage() / cores).min(100.0)
                                },
                                memory: p.memory(),
                                kind: if apps.contains(&pid) {
                                    ProcKind::App
                                } else {
                                    ProcKind::Background
                                },
                                protected: pid == me
                                    || pid <= 4
                                    || CRITICAL.contains(&name_lower.as_str()),
                                name,
                                name_lower,
                            }
                        })
                        .collect();
                    generation += 1;
                    if let Ok(mut g) = shared.lock() {
                        *g = ProcSnapshot {
                            generation,
                            rows: Arc::new(rows),
                            error: None,
                        };
                    }
                    repaint();
                    // Sample again quickly after the baseline, then every 2 s.
                    let wait_ms = if generation == 1 { 400 } else { 2000 };
                    for _ in 0..wait_ms / 100 {
                        if stop_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
        if let Err(e) = spawned
            && let Ok(mut g) = snapshot.lock()
        {
            g.error = Some(trf("Could not start the process monitor: {0}", &[&e]));
        }

        Self { snapshot, stop }
    }

    /// The latest sample. Cheap: the rows are shared, not copied.
    pub fn latest(&self) -> ProcSnapshot {
        self.snapshot.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for ProcessMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_lists_processes() {
        let monitor = ProcessMonitor::start(|| {});
        std::thread::sleep(Duration::from_millis(1500));
        let snap = monitor.latest();
        assert!(snap.generation >= 2, "expected two samples");
        assert!(snap.rows.len() > 10);
        let me = snap
            .rows
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("own process listed");
        assert!(me.protected);
        assert!(
            snap.rows
                .iter()
                .any(|p| p.name_lower == "csrss.exe" && p.protected)
        );
        assert!(is_running("explorer.exe"));
    }
}
