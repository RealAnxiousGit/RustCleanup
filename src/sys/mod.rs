//! Everything that talks to Windows. Nothing in here touches the UI, and
//! every fallible call returns `Result` instead of panicking.

pub mod disk;
pub mod power;
pub mod processes;
pub mod ram;
pub mod registry;
pub mod tools;
pub mod tweaks;
pub mod util;
