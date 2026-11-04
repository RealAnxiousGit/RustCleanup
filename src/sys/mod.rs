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
