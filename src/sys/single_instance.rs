//! Only one RustCleanup runs at a time. Opening the exe again brings the
//! running copy's window back instead of starting a second copy.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
};
use windows::core::w;

use super::tray::{SHOW_MESSAGE, WINDOW_CLASS};

/// Holds the instance mutex for the life of the process.
pub struct InstanceLock(HANDLE);

// SAFETY: the handle is only kept alive, never used from several threads.
unsafe impl Send for InstanceLock {}

impl Drop for InstanceLock {
    /// Lets a copy started by this one (e.g. the OpenGL fallback) take over.
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is released once.
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

fn try_lock() -> Option<InstanceLock> {
    // SAFETY: creating a named mutex; the handle is kept until the process exits.
    unsafe {
        let handle = CreateMutexW(None, true, w!("Local\\RustCleanup.SingleInstance")).ok()?;
        let error = GetLastError();
        // ACCESS_DENIED: a copy running with different rights already owns it.
        if error == ERROR_ALREADY_EXISTS || error == ERROR_ACCESS_DENIED {
            return None;
        }
        Some(InstanceLock(handle))
    }
}

/// Becomes the only running copy, or asks the running copy to show itself
/// and returns `None` (the caller then exits).
///
/// `wait` is used after an update or restart, while the previous copy is
/// still shutting down: it keeps trying for a few seconds instead of handing over.
pub fn acquire(wait: bool) -> Option<InstanceLock> {
    let deadline = Instant::now() + Duration::from_secs(if wait { 15 } else { 0 });
    loop {
        if let Some(lock) = try_lock() {
            return Some(lock);
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    show_running_copy();
    None
}

/// Posts "show yourself" to the running copy's tray window.
fn show_running_copy() {
    // The first copy may still be starting up, so look for its window a few times.
    for _ in 0..25 {
        // SAFETY: plain window lookups and a posted message; a stale handle just fails.
        unsafe {
            if let Ok(hwnd) = FindWindowW(WINDOW_CLASS, None) {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                // Lets the running copy bring its window to the front.
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(hwnd), SHOW_MESSAGE, WPARAM(0), LPARAM(0));
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
