//! Everything that talks to Windows. Nothing in here touches the UI, and
//! every fallible call returns `Result` instead of panicking.

// Shared plumbing
pub mod command;
pub mod format;
pub mod privilege;
pub mod registry;
pub mod shell;

// One module per feature
pub mod chkdsk;
pub mod copilot;
pub mod disk;
pub mod memory;
pub mod power;
pub mod process_actions;
pub mod process_monitor;
pub mod telemetry;
pub mod tools;
pub mod tray;
pub mod updates;
