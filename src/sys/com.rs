//! COM setup shared by the Task Scheduler, shell and UI Automation helpers.

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

/// Keeps COM initialized on the current thread while it is alive.
///
/// If the thread already joined a different apartment (the UI thread, say),
/// COM is usable as it is, so nothing is undone on drop.
pub struct Apartment {
    owned: bool,
}

impl Apartment {
    pub fn enter() -> Result<Self, String> {
        // SAFETY: paired with CoUninitialize in Drop only when this call took a reference.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self { owned: false });
        }
        hr.ok().map_err(|e| format!("COM: {}", e.message()))?;
        Ok(Self { owned: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: balances the successful CoInitializeEx above, on the same thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// A short readable message for a failed COM call.
pub fn message(e: &windows::core::Error) -> String {
    let text = e.message();
    if text.trim().is_empty() {
        format!("{:#010X}", e.code().0)
    } else {
        text.trim().to_string()
    }
}
