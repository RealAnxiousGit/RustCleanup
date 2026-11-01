//! The interface. Each section of the main window and each detached window
//! has its own module; `app.rs` only wires them together.

pub mod activity;
pub mod credits;
pub mod disk;
pub mod memory;
pub mod tasks;
pub mod theme;
pub mod tweaks;
pub mod widgets;

use std::sync::{Arc, OnceLock};

use eframe::egui::IconData;

/// The window icon, decoded once and shared by every window.
pub fn app_icon() -> Arc<IconData> {
    static ICON: OnceLock<Arc<IconData>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(
            eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png"))
                .unwrap_or_default(),
        )
    })
    .clone()
}
