//! Shell helpers: restarting Explorer, opening links as the signed-in user,
//! finding or showing our own windows, and reading shell preferences.

use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoCreateInstance, IDispatch, IServiceProvider,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellView, IShellWindows,
    SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, SPI_GETCLIENTAREAANIMATION,
    SW_RESTORE, SW_SHOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, ShowWindow,
    SystemParametersInfoW,
};
use windows::core::{BOOL, BSTR, Interface};

use super::com::{self, Apartment};
use super::command::{run_hidden, windows_dir};
use super::{process_monitor, tasks};
use super::registry::{self, Hive};
use crate::events::Logger;
use crate::i18n::{tr, trf};

/// Restarts explorer.exe so shell policies apply without signing out.
///
/// Explorer is ended, then started again as the signed-in user. Winlogon
/// used to bring it back by itself (`AutoRestartShell`), but current
/// Windows 11 builds leave the desktop empty after it is ended, so it is only
/// given a moment to do so. Starting explorer.exe straight from RustCleanup
/// would make the shell inherit the admin token; that is the last resort.
pub fn restart_explorer(log: &Logger) -> Result<(), String> {
    const EXPLORER: &str = "explorer.exe";
    log.info(tr("Restarting Explorer to apply the change."));
    let old = process_monitor::pids_named(EXPLORER);
    let out = run_hidden("taskkill.exe", &["/F", "/IM", EXPLORER])?;
    if !out.success {
        log.warn(format!("taskkill explorer.exe: {}", out.summary()));
    }
    // A new Explorer is one whose PID was not there before.
    let came_back = |ticks: u32| {
        (0..ticks).any(|_| {
            std::thread::sleep(Duration::from_millis(250));
            !process_monitor::pids_named(EXPLORER).is_subset(&old)
        })
    };
    // Wait for the old one to be gone before deciding it did not come back.
    for _ in 0..20 {
        if process_monitor::pids_named(EXPLORER).is_disjoint(&old) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if came_back(8) {
        log.info(tr("Explorer restarted."));
        return Ok(());
    }
    let program = windows_dir().join(EXPLORER);
    let dir = windows_dir();
    match tasks::run_as_user(&program.to_string_lossy(), "", &dir.to_string_lossy()) {
        Ok(()) if came_back(20) => {
            log.info(tr("Explorer restarted."));
            return Ok(());
        }
        Ok(()) => log.warn("Explorer did not start from its task; starting it directly."),
        Err(e) => log.warn(format!("Explorer task: {e}")),
    }
    std::process::Command::new(program)
        .spawn()
        .map_err(|e| {
            trf(
                "Explorer did not restart and could not be started: {0}",
                &[&e],
            )
        })?;
    log.warn(tr(
        "Explorer was started manually. Sign out and back in if the taskbar misbehaves.",
    ));
    Ok(())
}

/// Opens a URL or program through Explorer so it runs as the signed-in user.
///
/// RustCleanup runs elevated. Launching a browser directly would hand it our
/// admin token; passing the target to explorer.exe starts it unelevated.
pub fn open_unelevated(target: &str) -> Result<(), String> {
    std::process::Command::new(windows_dir().join("explorer.exe"))
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {target}: {e}"))
}

/// Starts a program as the signed-in user, without RustCleanup's admin token.
///
/// `open_unelevated` can't be used for programs: explorer.exe treats
/// `cmd.exe` as a path and opens a folder instead. This asks the desktop's
/// own Explorer window to run it (`IShellDispatch2::ShellExecute`), so it
/// starts exactly as if the user had launched it from the Start menu. With no
/// desktop Explorer running, a one-off scheduled task starts it instead.
pub fn run_unelevated(program: &str, args: &str, dir: &str) -> Result<(), String> {
    let (program, args, dir) = (program.to_string(), args.to_string(), dir.to_string());
    // A fresh thread gets its own apartment whatever the caller's thread uses.
    std::thread::spawn(move || {
        let _com = Apartment::enter()?;
        // SAFETY: plain COM calls; every result is checked.
        unsafe {
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)
                .map_err(|e| com::message(&e))?;
            let desktop = VARIANT::from(CSIDL_DESKTOP as i32);
            let none = VARIANT::default();
            let mut hwnd = 0i32;
            let shell = windows
                .FindWindowSW(&desktop, &none, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)
                .and_then(|d| d.cast::<IServiceProvider>())
                .and_then(|sp| sp.QueryService::<IShellBrowser>(&SID_STopLevelBrowser))
                .and_then(|browser| browser.QueryActiveShellView())
                .and_then(|view: IShellView| view.GetItemObject::<IDispatch>(SVGIO_BACKGROUND))
                .and_then(|view| view.cast::<IShellFolderViewDual>())
                .and_then(|folder| folder.Application())
                .and_then(|app| app.cast::<IShellDispatch2>())
                .map_err(|e| com::message(&e));
            let Ok(shell) = shell else {
                // No desktop to ask (Explorer is not running): use Task Scheduler.
                return tasks::run_as_user(&program, &args, &dir)
                    .map_err(|e| format!("the desktop shell is not available: {e}"));
            };
            shell
                .ShellExecute(
                    &BSTR::from(program.as_str()),
                    &VARIANT::from(args.as_str()),
                    &VARIANT::from(dir.as_str()),
                    &VARIANT::from(""),
                    &VARIANT::from(1i32), // SW_SHOWNORMAL
                )
                .map_err(|e| com::message(&e))
        }
    })
    .join()
    .unwrap_or_else(|_| Err("the launcher thread stopped".into()))
}

/// A window handle that can cross threads (HWND itself is not `Send`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowId(pub isize);

impl WindowId {
    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// Finds this process's top-level window with exactly this title.
pub fn find_own_window(title: &str) -> Option<WindowId> {
    struct Search<'a> {
        pid: u32,
        title: &'a str,
        found: Option<WindowId>,
    }
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the &mut Search passed below, alive for the whole enumeration.
        unsafe {
            let search = &mut *(lparam.0 as *mut Search);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == search.pid {
                let mut buf = [0u16; 128];
                let len = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
                if String::from_utf16_lossy(&buf[..len]) == search.title {
                    search.found = Some(WindowId(hwnd.0 as isize));
                    return BOOL(0); // stop enumerating
                }
            }
            BOOL(1)
        }
    }
    let mut search = Search {
        pid: std::process::id(),
        title,
        found: None,
    };
    // SAFETY: the callback only touches `search` during this synchronous call.
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}

/// Shows, restores and focuses a window (brings the app back from the tray).
pub fn show_window(id: WindowId) {
    let hwnd = id.hwnd();
    // SAFETY: plain window-management calls; a stale handle just makes them fail.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// False when "Animation effects" is turned off in Windows settings, in which
/// case the theme switches instantly instead of fading.
pub fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer.
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    result.is_err() || enabled.as_bool()
}

/// True if Windows is set to light mode for apps.
pub fn system_prefers_light() -> bool {
    registry::get_dword(
        Hive::CurrentUser,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "AppsUseLightTheme",
    ) == Some(1)
}
