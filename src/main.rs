//! RustCleanup — a Windows 11 optimization and debloating utility.
//!
//! The UAC manifest embedded by `build.rs` makes Windows ask for
//! Administrator rights on launch, which every tweak in here needs.

// No console window behind the GUI in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod events;
mod sys;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RustCleanup")
        .with_inner_size([1060.0, 820.0])
        .with_min_inner_size([820.0, 580.0]);
    // The icon is optional: a decode failure just means the default icon.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("RustCleanup", options, Box::new(|cc| Ok(Box::new(app::RustCleanupApp::new(cc)))))
}
