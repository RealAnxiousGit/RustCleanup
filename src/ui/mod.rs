//! The interface. Each section of the main window and each detached window
//! has its own module; `app.rs` only wires them together.
//!
//! Detached windows (Scan Disk, task list, Settings, Credits, update) are
//! egui "deferred" viewports: each is its own native window that paints on
//! its own schedule, so it keeps working while the main window is hidden in
//! the tray. Their state is shared with the main window behind a mutex.

pub mod activity;
pub mod changelog;
pub mod credits;
pub mod disk;
pub mod flags;
pub mod memory;
pub mod settings;
pub mod tasks;
pub mod theme;
pub mod tweaks;
pub mod update;
pub mod widgets;

use std::sync::{Arc, OnceLock};

use eframe::egui::{self, IconData, ViewportBuilder, ViewportCommand, ViewportId};

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

/// Every detached window, by viewport name.
pub const WINDOWS: [&str; 6] = [
    disk::WINDOW,
    tasks::WINDOW,
    settings::WINDOW,
    credits::WINDOW,
    update::WINDOW,
    changelog::WINDOW,
];

pub fn window_id(name: &str) -> ViewportId {
    ViewportId::from_hash_of(name)
}

/// Repaints every detached window, e.g. while the theme fades or after the
/// language changes. Windows that aren't open just ignore it.
pub fn repaint_windows(ctx: &egui::Context) {
    for name in WINDOWS {
        ctx.request_repaint_of(window_id(name));
    }
}

/// Brings an already open window to the front.
pub fn focus_window(ctx: &egui::Context, name: &str) {
    let id = window_id(name);
    ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
}

/// A detached window's frame: title, size and icon.
pub fn window_builder(title: &str, size: [f32; 2], min: [f32; 2]) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size(min)
        .with_icon(app_icon())
}

/// True when the window's close button or Escape was pressed.
pub fn close_pressed(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
}

/// Tells the main window a detached window closed, so it stops showing it.
pub fn closed(ctx: &egui::Context) {
    ctx.request_repaint_of(ViewportId::ROOT);
}
