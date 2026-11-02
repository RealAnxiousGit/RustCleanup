//! The Credits window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Margin, RichText, Ui};

use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::events::Logger;
use crate::i18n::{n_, tr, trf};

pub const WINDOW: &str = "credits_window";
const REPO: &str = "https://github.com/RealAnxiousGit/RustCleanup";

/// (name, what it was used for, link)
const TOOLKITS: [(&str, &str, &str); 2] = [
    (
        "graphify",
        n_("Code graph used to plan and check the module layout"),
        "https://github.com/Graphify-Labs/graphify",
    ),
    (
        "anti-slop",
        n_("Design rules applied to the interface"),
        "https://github.com/miqdadbadjuber/anti-slop",
    ),
];

const LIBRARIES: [(&str, &str); 5] = [
    ("egui and eframe", "https://github.com/emilk/egui"),
    ("sysinfo", "https://github.com/GuillaumeGomez/sysinfo"),
    ("windows-rs", "https://github.com/microsoft/windows-rs"),
    ("winreg", "https://github.com/gentoo90/winreg-rs"),
    (
        "embed-resource",
        "https://github.com/nabijaczleweli/rust-embed-resource",
    ),
];

pub struct Credits {
    open: Arc<AtomicBool>,
    log: Logger,
}

impl Credits {
    pub fn new(log: Logger) -> Self {
        Self {
            open: Arc::default(),
            log,
        }
    }

    pub fn open(&self, ctx: &egui::Context) {
        if self.open.swap(true, Ordering::Relaxed) {
            super::focus_window(ctx, WINDOW);
        }
    }

    fn person(ui: &mut Ui, pal: &Palette, role: &str, name: &str) {
        ui.label(RichText::new(role).size(12.5).color(pal.muted));
        ui.label(RichText::new(name).family(semibold()).size(16.0));
        ui.add_space(8.0);
    }

    /// Keeps the window alive. Called on every pass of the main window.
    pub fn show(&self, ctx: &egui::Context) {
        if !self.open.load(Ordering::Relaxed) {
            return;
        }
        let open = self.open.clone();
        let log = self.log.clone();
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(tr("Credits"), [460.0, 700.0], [360.0, 420.0]),
            move |ui, _class| {
                let pal = theme::palette();
                let mut close = super::close_pressed(ui.ctx());
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::same(22)),
                    )
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("RustCleanup").family(semibold()).size(22.0),
                                );
                                widgets::note(
                                    ui,
                                    &pal,
                                    trf("Version {0}", &[&env!("CARGO_PKG_VERSION")]),
                                );
                                ui.add_space(14.0);

                                Self::person(ui, &pal, tr("Programming"), "Claude Opus 5.5");
                                Self::person(ui, &pal, tr("Prompt Engineer"), "Nicholas Slaughter");

                                ui.separator();
                                widgets::heading(ui, tr("Toolkits"));
                                for (name, used_for, url) in TOOLKITS {
                                    widgets::link(ui, &log, name, url);
                                    widgets::note(ui, &pal, tr(used_for));
                                    ui.add_space(4.0);
                                }

                                ui.separator();
                                widgets::heading(ui, tr("Built with"));
                                for (name, url) in LIBRARIES {
                                    widgets::link(ui, &log, name, url);
                                }

                                ui.separator();
                                widgets::heading(ui, tr("Source code"));
                                widgets::link(
                                    ui,
                                    &log,
                                    "github.com/RealAnxiousGit/RustCleanup",
                                    REPO,
                                );
                                widgets::note(ui, &pal, tr("MIT licensed."));
                                ui.add_space(12.0);
                                if widgets::button(ui, tr("Close"), true).clicked() {
                                    close = true;
                                }
                            });
                    });
                if close {
                    open.store(false, Ordering::Relaxed);
                    super::closed(ui.ctx());
                }
            },
        );
    }
}
