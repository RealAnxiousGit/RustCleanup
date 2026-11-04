//! Everything that talks to the operating system. Nothing in here touches
//! the UI, and every fallible call returns `Result` instead of panicking.
//!
//! The Linux build has its own modules in `linux/`, with the same names and
//! the same public functions, so the UI uses `crate::sys::memory` and the
//! rest on both systems.

// Shared plumbing
#[cfg(windows)]
pub mod com;
#[cfg(windows)]
pub mod command;
pub mod format;
pub mod mail;
#[cfg(windows)]
pub mod privilege;
#[cfg(windows)]
pub mod registry;
#[cfg(windows)]
pub mod shell;

// One module per feature
#[cfg(all(test, windows))]
mod admin_tests;
#[cfg(windows)]
pub mod chkdsk;
#[cfg(windows)]
pub mod copilot;
#[cfg(all(test, windows))]
mod desktop_tests;
#[cfg(windows)]
pub mod disk;
#[cfg(windows)]
pub mod memory;
#[cfg(windows)]
pub mod power;
#[cfg(windows)]
pub mod process_actions;
#[cfg(windows)]
pub mod process_monitor;
#[cfg(windows)]
pub mod single_instance;
#[cfg(windows)]
pub mod startup;
#[cfg(any(debug_assertions, feature = "stress-test"))]
pub mod stress;
#[cfg(windows)]
pub mod storage_page;
#[cfg(windows)]
pub mod tasks;
#[cfg(windows)]
pub mod telemetry;
#[cfg(windows)]
pub mod tools;
#[cfg(windows)]
pub mod tray;
pub mod updater;
#[cfg(windows)]
pub mod updates;
#[cfg(windows)]
pub mod video;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    chkdsk, command, disk, memory, power, privilege, process_actions, process_monitor, registry,
    shell, single_instance, startup, storage_page, tools, tray, user, video,
};

/// The rectangle, in physical pixels, of the monitor nearest to `point`
/// (also physical pixels).
#[cfg(windows)]
pub fn monitor_rect(point: eframe::egui::Pos2) -> Option<eframe::egui::Rect> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    let at = POINT { x: point.x.round() as i32, y: point.y.round() as i32 };
    // SAFETY: plain Win32 queries; `info` has its size set as required.
    unsafe {
        let monitor = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let r = info.rcMonitor;
        Some(eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(r.left as f32, r.top as f32),
            eframe::egui::pos2(r.right as f32, r.bottom as f32),
        ))
    }
}
