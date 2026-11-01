//! Notification-area (tray) icon with a right-click menu and balloon
//! notifications, built directly on `Shell_NotifyIconW`.
//!
//! The icon lives on its own thread with a hidden window and message loop,
//! so the menu keeps working while the main window is hidden. Menu picks are
//! delivered through the `on_command` callback.

use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFY_ICON_INFOTIP_FLAGS,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY,
    WM_LBUTTONDBLCLK, WM_NULL, WM_USER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

/// What the user picked in the tray menu (or by clicking the icon).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    OpenApp,
    CleanRam,
    OpenTaskList,
    Credits,
    Exit,
}

/// Severity of a balloon notification.
#[derive(Clone, Copy, Debug)]
pub enum NotifyKind {
    Info,
    Warning,
    Error,
}

const CALLBACK_MSG: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
// NOTIFYICON_VERSION_4 events, delivered in LOWORD(lParam).
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

const MENU: [(usize, PCWSTR, TrayCommand); 5] = [
    (1, w!("Open App"), TrayCommand::OpenApp),
    (2, w!("Clean RAM"), TrayCommand::CleanRam),
    (3, w!("Open task list"), TrayCommand::OpenTaskList),
    (4, w!("Credits"), TrayCommand::Credits),
    (5, w!("Exit"), TrayCommand::Exit),
];

/// State the window procedure needs. There is only ever one tray icon.
struct Shared {
    on_command: Box<dyn Fn(TrayCommand) + Send + Sync>,
    icon: isize,
    taskbar_created: u32,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// A live tray icon. Dropping it removes the icon and stops its thread.
pub struct Tray {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    /// Creates the tray icon. Fails if the shell has no notification area
    /// (for example when Explorer is not running).
    pub fn start(on_command: impl Fn(TrayCommand) + Send + Sync + 'static) -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<isize, String>>();
        let mut on_command = Some(on_command);
        let thread = std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                let hwnd = match create_window() {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let icon = load_icon();
                let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
                if let Some(cb) = on_command.take() {
                    let _ = SHARED.set(Shared {
                        on_command: Box::new(cb),
                        icon,
                        taskbar_created,
                    });
                }
                if !add_icon(hwnd) {
                    let _ = ready_tx.send(Err("the notification area is not available".into()));
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return;
                }
                let _ = ready_tx.send(Ok(hwnd.0 as isize));
                message_loop();
            })
            .map_err(|e| format!("could not start the tray thread: {e}"))?;

        match ready_rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the tray thread stopped unexpectedly".into()),
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    /// Shows a Windows notification from the tray icon.
    pub fn notify(&self, kind: NotifyKind, title: &str, body: &str) {
        let mut data = base_data(self.hwnd());
        data.uFlags = NIF_INFO;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, body);
        data.dwInfoFlags = match kind {
            NotifyKind::Info => NIIF_INFO,
            NotifyKind::Warning => NIIF_WARNING,
            NotifyKind::Error => NIIF_ERROR,
        } | NOTIFY_ICON_INFOTIP_FLAGS(0);
        // SAFETY: `data` is fully initialised and refers to our own icon.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// Removes the icon now. Used before a hard exit, where Drop won't run.
    pub fn remove(&mut self) {
        // SAFETY: deleting our own icon; harmless if already gone.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &base_data(self.hwnd()));
            let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

/// Copies `text` into a fixed-size UTF-16 buffer, truncating and NUL-terminating.
fn copy_wide<const N: usize>(dest: &mut [u16; N], text: &str) {
    let mut i = 0;
    for unit in text.encode_utf16().take(N - 1) {
        dest[i] = unit;
        i += 1;
    }
    dest[i] = 0;
}

fn create_window() -> Result<HWND, String> {
    // SAFETY: standard window-class registration and creation on this thread.
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.message().to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("RustCleanupTray"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // A real (never shown) top-level window rather than a message-only one,
        // because only top-level windows receive the TaskbarCreated broadcast.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RustCleanupTray"),
            w!("RustCleanup tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| format!("could not create the tray window: {}", e.message()))
    }
}

/// Loads the app icon embedded by resources.rc, at the small-icon size.
fn load_icon() -> isize {
    // SAFETY: loading a resource from our own module.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return 0;
        };
        LoadImageW(
            Some(instance.into()),
            w!("IDI_ICON1"),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
        .map(|h| h.0 as isize)
        .unwrap_or(0)
    }
}

fn add_icon(hwnd: HWND) -> bool {
    let mut data = base_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK_MSG;
    data.hIcon = HICON(SHARED.get().map(|s| s.icon).unwrap_or(0) as *mut _);
    copy_wide(&mut data.szTip, "RustCleanup");
    // SAFETY: `data` is fully initialised.
    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            return false;
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
    true
}

fn message_loop() {
    let mut msg = MSG::default();
    // SAFETY: standard message pump for this thread's windows.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn send(cmd: TrayCommand) {
    if let Some(shared) = SHARED.get() {
        (shared.on_command)(cmd);
    }
}

/// Shows the right-click menu at the cursor and returns the picked command.
fn show_menu(hwnd: HWND) -> Option<TrayCommand> {
    // SAFETY: the menu is created, tracked and destroyed on this thread.
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (id, label, cmd) in MENU {
            if cmd == TrayCommand::Exit {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            }
            let _ = AppendMenuW(menu, MF_STRING, id, label);
        }
        let _ = SetMenuDefaultItem(menu, 1, 0);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        MENU.iter()
            .find(|(id, _, _)| *id as i32 == picked.0)
            .map(|(_, _, cmd)| *cmd)
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        CALLBACK_MSG => {
            match (lparam.0 as u32) & 0xFFFF {
                WM_CONTEXTMENU => {
                    if let Some(cmd) = show_menu(hwnd) {
                        send(cmd);
                    }
                }
                NIN_SELECT | NIN_KEYSELECT | WM_LBUTTONDBLCLK | NIN_BALLOONUSERCLICK => {
                    send(TrayCommand::OpenApp)
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: destroying our own window on its thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // Explorer restarted (e.g. after the Copilot toggle): put the icon back.
        m if SHARED
            .get()
            .is_some_and(|s| s.taskbar_created != 0 && s.taskbar_created == m) =>
        {
            add_icon(hwnd);
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_icon_loads() {
        // resources.rc is linked into the test binary too, so this proves the
        // tray will show the real icon rather than a blank one.
        assert_ne!(super::load_icon(), 0);
    }
}
