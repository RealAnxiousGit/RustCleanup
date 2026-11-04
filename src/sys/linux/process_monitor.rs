//! Background sampling of running processes for the task list, from /proc
//! through sysinfo.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use crate::i18n::trf;

/// Processes that keep Linux and the desktop alive. Stopping or killing any
/// of these can freeze the session or the machine, so the task list
/// refuses to. Names are as the kernel reports them, cut to 15 characters.
const CRITICAL: [&str; 19] = [
    "systemd",
    "init",
    "kthreadd",
    "systemd-journal",
    "systemd-udevd",
    "systemd-logind",
    "dbus-daemon",
    "dbus-broker",
    "dbus-broker-lau",
    "xorg",
    "xwayland",
    "gnome-shell",
    "kwin_wayland",
    "kwin_x11",
    "gdm",
    "gdm-session-wor",
    "sddm",
    "lightdm",
    "weston",
];

/// Parts of the desktop that can be ended (they are started again) but must
/// never be stopped: a frozen panel or sound server hangs the session.
const SYSTEM: [&str; 10] = [
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "pulseaudio",
    "plasmashell",
    "xfce4-panel",
    "xdg-desktop-por",
    "gnome-session-b",
    "polkitd",
    "networkmanager",
];

/// Session helpers that inherit the display but never show a window.
const HELPER_PREFIXES: [&str; 24] = [
    "gsd-", "xdg-", "ibus", "at-spi", "evolution-", "tracker", "gvfs", "goa-", "dconf", "dbus",
    "gnome-shell", "gnome-session", "gnome-keyring", "kded", "kglobal", "ksmserver", "kactivity", "baloo", "fcitx", "xembed", "polkit",
    "pipewire", "wireplumber", "systemd",
];

/// The kernel keeps only the first 15 bytes of a process name. When the
/// name fills them, the program's (or script's) file name from its command
/// line is used instead, as long as it starts with the same text.
fn full_name(pid: u32, comm: String) -> String {
    if comm.len() < 15 {
        return comm;
    }
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .and_then(|raw| {
            // The program, or the script an interpreter runs (python3 /usr/bin/x).
            raw.split(|&b| b == 0).take(3).find_map(|arg| {
                let arg = String::from_utf8_lossy(arg);
                let base = arg.rsplit('/').next()?.split(' ').next()?;
                (base.len() > comm.len() && base.starts_with(&comm)).then(|| base.to_owned())
            })
        })
        .unwrap_or(comm)
}

/// Whether a process shows a window of its own.
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
    pub name_lower: String,
    /// CPU usage as a share of the whole machine (0 to 100).
    pub cpu: f32,
    pub memory: u64,
    pub kind: ProcKind,
    /// System-critical or RustCleanup itself: only memory trimming is allowed.
    pub protected: bool,
    /// A desktop component: it can be optimized and ended (with a warning)
    /// but not suspended.
    pub system: bool,
    /// Stopped (SIGSTOP), by RustCleanup or anything else.
    pub suspended: bool,
}

#[derive(Clone, Default)]
pub struct ProcSnapshot {
    pub generation: u64,
    pub rows: Arc<Vec<ProcInfo>>,
    pub error: Option<String>,
}

/// True when the environment of `pid` names a display (X11 or Wayland).
fn has_display(pid: u32) -> bool {
    std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|env| {
        env.split(|&b| b == 0)
            .any(|v| v.starts_with(b"DISPLAY=") || v.starts_with(b"WAYLAND_DISPLAY="))
    })
}

/// The controlling terminal number from `/proc/<pid>/stat` (0 for none).
fn tty_of(pid: u32) -> i64 {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            let rest = &s[s.rfind(')')? + 2..];
            rest.split_whitespace().nth(4)?.parse().ok()
        })
        .unwrap_or(0)
}

/// Samples the process list on its own thread while the task list is open.
/// Dropping the monitor stops the thread.
pub struct ProcessMonitor {
    snapshot: Arc<Mutex<ProcSnapshot>>,
    stop: Arc<AtomicBool>,
    wake: Arc<AtomicBool>,
}

impl ProcessMonitor {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        let snapshot = Arc::new(Mutex::new(ProcSnapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(AtomicBool::new(false));
        let (shared, stop_flag, wake_flag) = (snapshot.clone(), stop.clone(), wake.clone());

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
                let refresh = ProcessRefreshKind::nothing()
                    .with_cpu()
                    .with_memory()
                    .with_exe(UpdateKind::OnlyIfNotSet)
                    .without_tasks();
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1) as f32;
                let me = std::process::id();
                // The AppImage runtime holds RustCleanup's own files.
                let parent = std::os::unix::process::parent_id();
                let protect_parent = std::env::var_os("APPIMAGE").is_some();
                let mut generation = 0u64;
                // Whether a pid is an app is decided once: environ reads are slow.
                let mut app_cache: std::collections::HashMap<u32, bool> = Default::default();
                // Full names of processes whose kernel name was cut at 15 bytes.
                let mut name_cache: std::collections::HashMap<u32, String> = Default::default();

                while !stop_flag.load(Ordering::Relaxed) {
                    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
                    let procs = sys.processes();
                    app_cache.retain(|pid, _| procs.contains_key(&Pid::from_u32(*pid)));
                    name_cache.retain(|pid, _| procs.contains_key(&Pid::from_u32(*pid)));
                    let rows: Vec<ProcInfo> = procs
                        .values()
                        .filter(|p| p.thread_kind().is_none())
                        .filter(|p| {
                            // Kernel threads: kthreadd (2) and its children.
                            let pid = p.pid().as_u32();
                            pid != 2 && p.parent().is_none_or(|pp| pp.as_u32() != 2)
                        })
                        .map(|p| {
                            let pid = p.pid().as_u32();
                            let comm = p.name().to_string_lossy().into_owned();
                            // name_lower stays the kernel name, which the lists above match.
                            let name_lower = comm.to_lowercase();
                            let name = name_cache
                                .entry(pid)
                                .or_insert_with(|| full_name(pid, comm))
                                .clone();
                            let is_app = *app_cache.entry(pid).or_insert_with(|| {
                                let exe = p.exe().map(|e| e.to_string_lossy().into_owned());
                                let same_as_parent = p
                                    .parent()
                                    .and_then(|pp| procs.get(&pp))
                                    .is_some_and(|pp| pp.name() == p.name());
                                !same_as_parent
                                    && pid != me
                                    && !HELPER_PREFIXES.iter().any(|h| name_lower.starts_with(h))
                                    && !exe.is_some_and(|e| e.contains("/libexec/"))
                                    && tty_of(pid) == 0
                                    && has_display(pid)
                            });
                            ProcInfo {
                                pid,
                                cpu: if generation == 0 {
                                    0.0
                                } else {
                                    (p.cpu_usage() / cores).min(100.0)
                                },
                                memory: p.memory(),
                                kind: if is_app {
                                    ProcKind::App
                                } else {
                                    ProcKind::Background
                                },
                                protected: pid == me
                                    || pid == 1
                                    || (protect_parent && pid == parent)
                                    || CRITICAL.contains(&name_lower.as_str()),
                                system: SYSTEM.contains(&name_lower.as_str()),
                                suspended: matches!(p.status(), ProcessStatus::Stop),
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
                    let wait_ms = if generation == 1 { 400 } else { 2000 };
                    for _ in 0..wait_ms / 100 {
                        if stop_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        if wake_flag.swap(false, Ordering::Relaxed) {
                            break;
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

        Self {
            snapshot,
            stop,
            wake,
        }
    }

    pub fn waker(&self) -> Arc<AtomicBool> {
        self.wake.clone()
    }

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
        assert!(snap.rows.len() > 3);
        let me = snap
            .rows
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("own process listed");
        assert!(me.protected);
        assert!(snap.rows.iter().any(|p| p.pid == 1 && p.protected));
    }
}
