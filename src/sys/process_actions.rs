//! Actions on a single process: optimize, suspend, resume, restart and end.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, c_void};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, OpenProcess, OpenProcessToken, PROCESS_ACCESS_RIGHTS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME,
    PROCESS_TERMINATE, SetPriorityClass, TerminateProcess,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use super::privilege::OwnedHandle;
use super::shell;

// NtSuspendProcess / NtResumeProcess suspend every thread of a process at
// once; they are the same calls Resource Monitor uses.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(handle: HANDLE) -> i32;
    fn NtResumeProcess(handle: HANDLE) -> i32;
    fn NtQuerySystemInformation(class: i32, info: *mut c_void, len: u32, needed: *mut u32) -> i32;
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

/// Suspends a process, unless it already is: every suspend needs its own
/// resume, so suspending twice would leave it frozen after one Resume.
pub fn suspend(pid: u32) -> Result<(), String> {
    if suspended_pids().contains(&pid) {
        return Ok(());
    }
    let handle = open(pid, PROCESS_SUSPEND_RESUME)?;
    // SAFETY: valid handle with PROCESS_SUSPEND_RESUME access.
    nt_result(unsafe { NtSuspendProcess(handle.0) })
}

/// Resumes a process fully, even if it was suspended more than once.
pub fn resume(pid: u32) -> Result<(), String> {
    let handle = open(pid, PROCESS_SUSPEND_RESUME)?;
    for _ in 0..16 {
        // SAFETY: valid handle with PROCESS_SUSPEND_RESUME access.
        nt_result(unsafe { NtResumeProcess(handle.0) })?;
        if !suspended_pids().contains(&pid) {
            break;
        }
    }
    Ok(())
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

/// PIDs whose threads are all suspended, the way Task Manager decides it.
/// This also catches processes suspended by Windows or another program.
pub fn suspended_pids() -> HashSet<u32> {
    // SystemProcessInformation: one SYSTEM_PROCESS_INFORMATION per process,
    // each followed by a SYSTEM_THREAD_INFORMATION per thread (64-bit layout).
    const CLASS: i32 = 5;
    const PROCESS_SIZE: usize = 0x100;
    const PID_OFFSET: usize = 0x50;
    const THREAD_SIZE: usize = 0x50;
    const THREAD_STATE: usize = 0x44;
    const WAIT_REASON: usize = 0x48;
    const WAITING: u32 = 5;
    const SUSPENDED: u32 = 5;

    let mut buf: Vec<u64> = vec![0; 128 * 1024];
    let mut status = -1;
    for _ in 0..4 {
        let mut needed = 0u32;
        let bytes = (buf.len() * 8) as u32;
        // SAFETY: the buffer is writable for `bytes` bytes.
        status = unsafe { NtQuerySystemInformation(CLASS, buf.as_mut_ptr().cast(), bytes, &mut needed) };
        if status >= 0 {
            break;
        }
        // Too small: grow with room for processes started meanwhile.
        buf = vec![0; (needed as usize).max(bytes as usize * 2) / 8 + 8192];
    }
    let mut found = HashSet::new();
    if status < 0 {
        return found;
    }
    let base = buf.as_ptr() as *const u8;
    let len = buf.len() * 8;
    let read_u32 = |at: usize| -> u32 {
        // SAFETY: every caller checks `at + 4 <= len` first.
        unsafe { (base.add(at) as *const u32).read_unaligned() }
    };
    let mut at = 0usize;
    loop {
        if at + PROCESS_SIZE > len {
            break;
        }
        let next = read_u32(at) as usize;
        let threads = read_u32(at + 4) as usize;
        // SAFETY: inside the entry checked above.
        let pid = unsafe { (base.add(at + PID_OFFSET) as *const usize).read_unaligned() } as u32;
        let first = at + PROCESS_SIZE;
        if threads > 0 && pid != 0 && first + threads * THREAD_SIZE <= len {
            let all = (0..threads).all(|i| {
                let t = first + i * THREAD_SIZE;
                read_u32(t + THREAD_STATE) == WAITING && read_u32(t + WAIT_REASON) == SUSPENDED
            });
            if all {
                found.insert(pid);
            }
        }
        if next == 0 {
            break;
        }
        at += next;
    }
    found
}

/// True if the process runs with an elevated (administrator) token.
fn is_elevated(pid: u32) -> bool {
    let Ok(process) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return false;
    };
    // SAFETY: plain token query with a correctly sized out-parameter.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(process.0, TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok()
            && elevation.TokenIsElevated != 0
    }
}

/// Joins arguments back into one command line, quoting where needed.
fn join_args(args: &[std::ffi::OsString]) -> String {
    args.iter()
        .map(|a| {
            let a = a.to_string_lossy();
            if a.is_empty() || a.contains([' ', '\t', '"']) {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How to start a program again after it was closed.
struct Relaunch {
    exe: PathBuf,
    args: String,
    dir: String,
    elevated: bool,
}

impl Relaunch {
    fn start(&self) -> Result<(), String> {
        let exe = self.exe.to_string_lossy();
        if !self.elevated {
            // As the signed-in user, not with RustCleanup's admin token.
            return shell::run_unelevated(&exe, &self.args, &self.dir);
        }
        // SAFETY: every string outlives the call.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(exe.as_ref()),
                &HSTRING::from(self.args.as_str()),
                &HSTRING::from(self.dir.as_str()),
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success; lower ones are SE_ERR_* codes.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(format!("error {}", result.0 as usize))
        }
    }
}

/// Closes each process and starts it again with the same command line.
///
/// Programs like browsers run as a tree of processes with one name. Picking
/// any of them restarts the whole program once, from its top process, so a
/// helper is never started on its own. Returns (name, result) per program.
pub fn restart(pids: &[u32]) -> Vec<(String, Result<(), String>)> {
    let Ok(mut sys) = System::new() else {
        return vec![(String::new(), Err("could not read the process list".into()))];
    };
    let kind = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_cwd(UpdateKind::Always);
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
        // Climb while the parent is the same program (and started first,
        // since Windows reuses PIDs).
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
        let Some(exe) = p.exe().map(PathBuf::from) else {
            results.push((name, Err("Windows did not say where its program file is".into())));
            continue;
        };
        let relaunch = Relaunch {
            args: join_args(p.cmd().get(1..).unwrap_or_default()),
            dir: p
                .cwd()
                .or_else(|| exe.parent())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
            elevated: is_elevated(root.as_u32()),
            exe,
        };

        // The whole tree of this program: the top process and its helpers.
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

/// Waits up to 5 seconds for a process to be gone, so a program that
/// allows only one copy of itself doesn't refuse the new one.
fn wait_for_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let alive = System::new().is_ok_and(|mut sys| {
            let target = Pid::from_u32(pid);
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[target]),
                true,
                ProcessRefreshKind::nothing(),
            );
            sys.process(target).is_some()
        });
        if !alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_arguments_with_quotes() {
        let args: Vec<std::ffi::OsString> =
            ["--a", "C:\\Program Files\\x", "", "say \"hi\""].map(Into::into).to_vec();
        assert_eq!(
            join_args(&args),
            "--a \"C:\\Program Files\\x\" \"\" \"say \\\"hi\\\"\""
        );
    }

    #[test]
    fn reads_suspended_processes() {
        // Nothing here is suspended; the query must still work and be quick.
        let start = Instant::now();
        let set = suspended_pids();
        assert!(!set.contains(&std::process::id()));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn suspend_resume_and_restart_a_child() {
        let mut child = std::process::Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("start ping");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(300));
        suspend(pid).unwrap();
        suspend(pid).unwrap(); // a second suspend is skipped
        assert!(suspended_pids().contains(&pid));
        resume(pid).unwrap();
        assert!(!suspended_pids().contains(&pid));
        suspend(pid).unwrap();
        terminate(pid).unwrap();
        let _ = child.wait();
    }
}
