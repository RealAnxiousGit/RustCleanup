//! RustCleanup: a Windows 11 and Linux optimization and debloating utility.
//!
//! The UAC manifest embedded by `build.rs` makes Windows ask for
//! Administrator rights on launch, which every change in here needs. On
//! Linux it starts as the user and can restart itself as root via pkexec.

// No console window behind the GUI in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod background;
mod events;
mod i18n;
mod settings;
mod sys;
mod ui;

use eframe::egui;

use crate::sys::single_instance;

/// Passed by a restart or update: wait for the old copy to exit.
pub const RESTARTED_ARG: &str = "--restarted";
/// Start in the tray (a restart while the window was hidden).
pub const HIDDEN_ARG: &str = "--hidden";
/// Hardware-accelerated UI failed to start, so use OpenGL this time.
const GLOW_FALLBACK_ARG: &str = "--opengl";
/// Linux: the root copy started by "Restart as root" asks the user's copy
/// to quit, then takes its place.
#[cfg(not(windows))]
pub const TAKE_OVER_ARG: &str = "--take-over";

fn main() -> eframe::Result {
    #[cfg(not(windows))]
    sys::user::prepare_session_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);

    // Development builds only: the scripted RAM stress test runs headless.
    #[cfg(any(debug_assertions, feature = "stress-test"))]
    if let Some(code) = sys::stress::cli(&args) {
        std::process::exit(code);
    }

    #[cfg(not(windows))]
    let taking_over = has(TAKE_OVER_ARG) && single_instance::ask_running_copy_to_quit();
    #[cfg(windows)]
    let taking_over = false;

    // A second copy just brings the first one forward and leaves.
    let Some(_instance) =
        single_instance::acquire(taking_over || has(RESTARTED_ARG) || has(GLOW_FALLBACK_ARG))
    else {
        return Ok(());
    };

    let s = settings::get();
    // A restart says for itself whether the window was hidden.
    let start_hidden =
        has(HIDDEN_ARG) || (s.start_minimized && !has(RESTARTED_ARG) && !taking_over);
    let hardware = s.hardware_ui && !has(GLOW_FALLBACK_ARG);

    let viewport = egui::ViewportBuilder::default()
        .with_title(app::MAIN_TITLE)
        .with_inner_size([1080.0, 800.0])
        .with_min_inner_size([860.0, 600.0])
        .with_icon(ui::app_icon())
        .with_visible(!start_hidden);
    // Lets the desktop match the window to its .desktop entry and icon.
    #[cfg(not(windows))]
    let viewport = viewport.with_app_id("rustcleanup");

    let options = eframe::NativeOptions {
        viewport,
        renderer: if hardware {
            eframe::Renderer::Wgpu
        } else {
            eframe::Renderer::Glow
        },
        ..Default::default()
    };
    let result = eframe::run_native(
        "RustCleanup",
        options,
        Box::new(move |cc| Ok(Box::new(app::RustCleanupApp::new(cc, start_hidden)))),
    );

    // Direct3D 12 (or Vulkan) can be missing on old drivers. eframe can only
    // start once per process, so try again as a new process with OpenGL.
    if hardware
        && result.is_err()
        && let Ok(exe) = sys::shell::app_exe()
    {
        drop(_instance);
        let mut cmd = std::process::Command::new(exe);
        cmd.arg(GLOW_FALLBACK_ARG);
        if start_hidden {
            cmd.arg(HIDDEN_ARG);
        }
        if cmd.spawn().is_ok() {
            return Ok(());
        }
    }
    result
}
