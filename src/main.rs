//! RustCleanup: a Windows 11 optimization and debloating utility.
//!
//! The UAC manifest embedded by `build.rs` makes Windows ask for
//! Administrator rights on launch, which every change in here needs.

// No console window behind the GUI in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod events;
mod sys;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title(app::MAIN_TITLE)
        .with_inner_size([1080.0, 800.0])
        .with_min_inner_size([860.0, 600.0])
        .with_icon(ui::app_icon());

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "RustCleanup",
        options,
        Box::new(|cc| Ok(Box::new(app::RustCleanupApp::new(cc)))),
    )
}
