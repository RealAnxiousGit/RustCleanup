//! Linux versions of the `sys` modules. Each file has the same name and the
//! same public functions as its Windows twin, so the UI code is shared.

pub mod chkdsk;
pub mod command;
pub mod disk;
pub mod memory;
pub mod power;
pub mod privilege;
pub mod process_actions;
pub mod process_monitor;
pub mod registry;
pub mod shell;
pub mod single_instance;
pub mod startup;
pub mod storage_page;
pub mod tools;
pub mod tray;
pub mod user;
pub mod video;
