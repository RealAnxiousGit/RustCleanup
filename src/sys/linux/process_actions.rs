//! Actions on a single process: optimize, suspend, resume, restart and end.
//!
//! - Optimize pages the process out (`process_madvise`), lowers its CPU
//!   priority (nice 10) and puts its disk access in the idle class.
//! - Suspend and Resume are `SIGSTOP` and `SIGCONT`.
//! - End asks politely with `SIGTERM` and follows up with `SIGKILL` if the
//!   process is still there three seconds later.
//! - Restart closes the program and starts it again with the same command
//!   line, as the user it ran as.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::user::user_command;

/// `ioprio_set` values: the idle I/O class for one process.
const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const IOPRIO_CLASS_IDLE: libc::c_int = 3;
const IOPRIO_CLASS_SHIFT: libc::c_int = 13;

fn errno() -> String {
    std::io::Error::last_os_error().to_string()
}

fn signal(pid: u32, sig: libc::c_int) -> Result<(), String> {
    if pid <= 1 {
        return Err("refusing to signal init".into());
    }
    // SAFETY: plain syscall.
    if unsafe { libc::kill(pid as libc::pid_t, sig) } == 0 {
        Ok(())
    } else {
        Err(errno())
    }
}

/// The start time from `/proc/<pid>/stat`, to tell a process from a later
/// one that got the same pid.
fn start_time(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &s[s.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// The state letter from `/proc/<pid>/stat` (`T` when stopped).
fn state(pid: u32) -> Option<char> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    s[s.rfind(')')? + 2..].chars().next()
}

/// "Optimize": pages the process out and lowers its CPU and disk priority
/// so it yields to whatever you are actively using.
pub fn optimize(pid: u32) -> Result<(), String> {
    let trimmed = super::memory::trim_process(pid);
    // SAFETY: plain syscalls on a pid.
    let reniced = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, 10) } == 0;
    let renice_err = errno();
    // SAFETY: plain syscall; a failure only leaves the I/O class as it was.
    unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            pid as libc::c_int,
            IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT,
        );
    }
    match (trimmed, reniced) {
        (_, true) | (Ok(()), _) => Ok(()),
        (Err(e), false) => Err(format!("{renice_err} ({e})")),
    }
}

/// Stops a process. Stopping twice is harmless on Linux.
pub fn suspend(pid: u32) -> Result<(), String> {
    if state(pid) == Some('T') {
        return Ok(());
    }
    signal(pid, libc::SIGSTOP)
}

pub fn resume(pid: u32) -> Result<(), String> {
    signal(pid, libc::SIGCONT)
}

/// Ends a process: `SIGTERM` now, `SIGKILL` in three seconds if it is still
/// running. Returns at once.
pub fn terminate(pid: u32) -> Result<(), String> {
    let started = start_time(pid);
    signal(pid, libc::SIGTERM)?;
    // A stopped process only sees SIGTERM once it runs again.
    let _ = signal(pid, libc::SIGCONT);
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if start_time(pid) != started || state(pid).is_none_or(|s| s == 'Z') {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if start_time(pid) == started {
            let _ = signal(pid, libc::SIGKILL);
        }
    });
    Ok(())
}

/// The effective uid of a process.
fn euid_of(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// True if the process runs as root.
pub fn is_elevated(pid: u32) -> bool {
    euid_of(pid) == Some(0)
}

/// `APPIMAGE` from a process's environment: an AppImage must be started
/// again from its file, not from its temporary mount.
fn appimage_of(pid: u32) -> Option<PathBuf> {
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    env.split(|&b| b == 0)
        .find_map(|v| v.strip_prefix(b"APPIMAGE="))
        .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
}

/// How to start a program again after it was closed.
struct Relaunch {
    exe: PathBuf,
    args: Vec<OsString>,
    dir: Option<PathBuf>,
    root: bool,
}

impl Relaunch {
    fn start(&self) -> Result<(), String> {
        let mut cmd = if self.root && super::privilege::is_elevated() {
            Command::new(&self.exe)
        } else {
            user_command(&self.exe)
        };
        cmd.args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        if let Some(dir) = self.dir.as_ref().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        super::shell::spawn_detached(&mut cmd).map_err(|e| e.to_string())
    }
}

/// Closes each program and starts it again with the same command line.
/// Picking any process of a program restarts the whole program once, from
/// its top process. Returns (name, result) per program.
pub fn restart(pids: &[u32]) -> Vec<(String, Result<(), String>)> {
    let Ok(mut sys) = System::new() else {
        return vec![(String::new(), Err("could not read the process list".into()))];
    };
    let kind = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_cwd(UpdateKind::Always)
        .without_tasks();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let procs = sys.processes();
    let name_of = |pid: Pid| procs.get(&pid).map(|p| p.name().to_os_string());

    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, p) in procs {
        if let Some(parent) = p.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }

    let mut roots: Vec<Pid> = Vec::new();
    for &pid in pids {
        let mut pid = Pid::from_u32(pid);
        let Some(name) = name_of(pid) else { continue };
        while let Some(parent) = procs.get(&pid).and_then(|p| p.parent())
            && name_of(parent).as_deref() == Some(name.as_os_str())
            && procs[&parent].start_time() <= procs[&pid].start_time()
        {
            pid = parent;
        }
        if !roots.contains(&pid) {
            roots.push(pid);
        }
    }

    let mut results = Vec::new();
    for root in roots {
        let p = &procs[&root];
        let name = p.name().to_string_lossy().into_owned();
        let exe = appimage_of(root.as_u32()).or_else(|| p.exe().map(PathBuf::from));
        let Some(exe) = exe.filter(|e| e.exists()) else {
            results.push((name, Err("Linux did not say where its program file is".into())));
            continue;
        };
        let relaunch = Relaunch {
            args: p.cmd().get(1..).unwrap_or_default().to_vec(),
            dir: p.cwd().map(PathBuf::from),
            root: is_elevated(root.as_u32()),
            exe,
        };

        let mut tree = vec![root];
        let mut i = 0;
        while i < tree.len() {
            for &child in children.get(&tree[i]).map(Vec::as_slice).unwrap_or_default() {
                if procs[&child].name() == OsStr::new(&name) && !tree.contains(&child) {
                    tree.push(child);
                }
            }
            i += 1;
        }
        if let Err(e) = terminate(root.as_u32()) {
            results.push((name, Err(e)));
            continue;
        }
        for &helper in &tree[1..] {
            let _ = terminate(helper.as_u32());
        }
        wait_for_exit(root.as_u32());
        results.push((name, relaunch.start()));
    }
    results
}

/// Waits up to 5 seconds for a process to be gone, so a single-instance
/// program does not refuse the new copy.
fn wait_for_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if state(pid).is_none_or(|s| s == 'Z') {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspend_resume_and_end_a_child() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start sleep");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(200));
        suspend(pid).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(state(pid), Some('T'));
        resume(pid).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_ne!(state(pid), Some('T'));
        assert!(!is_elevated(pid) || super::super::privilege::is_elevated());
        optimize(pid).unwrap();
        suspend(pid).unwrap();
        terminate(pid).unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
    }
}
