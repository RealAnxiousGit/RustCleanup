//! Everything that talks to Windows. Nothing in here touches the UI, and
//! every fallible call returns `Result` instead of panicking.

// Shared plumbing
pub mod com;
pub mod command;
pub mod format;
pub mod privilege;
pub mod registry;
pub mod shell;

// One module per feature
#[cfg(test)]
mod admin_tests;
pub mod chkdsk;
pub mod copilot;
#[cfg(test)]
mod desktop_tests;
pub mod disk;
pub mod memory;
pub mod power;
pub mod process_actions;
pub mod process_monitor;
pub mod single_instance;
pub mod startup;
pub mod storage_page;
pub mod tasks;
pub mod telemetry;
pub mod tools;
pub mod tray;
pub mod updater;
pub mod updates;
