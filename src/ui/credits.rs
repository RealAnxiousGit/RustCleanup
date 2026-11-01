//! The Credits window.

use eframe::egui::{self, Margin, RichText, Ui, ViewportBuilder, ViewportCommand, ViewportId};

use super::theme::{Palette, Theme, semibold};
use super::widgets;
use crate::events::Logger;

const REPO: &str = "https://github.com/RealAnxiousGit/RustCleanup";

/// (name, what it was used for, link)
const TOOLKITS: [(&str, &str, &str); 2] = [
    (
        "graphify",
        "Code graph used to plan and check the module layout",
        "https://github.com/Graphify-Labs/graphify",
    ),
    (
        "anti-slop",
        "Design rules applied to the interface",
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
    open: bool,
}

impl Credits {
    pub fn new() -> Self {
        Self { open: false }
    }

    pub fn open(&mut self, ctx: &egui::Context) {
        if self.open {
            ctx.send_viewport_cmd_to(
                ViewportId::from_hash_of("credits_window"),
                ViewportCommand::Focus,
            );
        }
        self.open = true;
    }

    fn person(ui: &mut Ui, pal: &Palette, role: &str, name: &str) {
        ui.label(RichText::new(role).size(12.5).color(pal.muted));
        ui.label(RichText::new(name).family(semibold()).size(16.0));
        ui.add_space(8.0);
    }

    pub fn window(&mut self, ctx: &egui::Context, theme: &Theme, log: &Logger) {
        if !self.open {
            return;
        }
        let pal = theme.palette;
        let builder = ViewportBuilder::default()
            .with_title("Credits")
            .with_inner_size([460.0, 700.0])
            .with_min_inner_size([360.0, 420.0])
            .with_icon(super::app_icon());

        let mut close = false;
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("credits_window"),
            builder,
            |ui, _class| {
                if ui
                    .ctx()
                    .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
                {
                    close = true;
                }
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
                                    format!("Version {}", env!("CARGO_PKG_VERSION")),
                                );
                                ui.add_space(14.0);

                                Self::person(ui, &pal, "Programming", "Claude Opus 5.5");
                                Self::person(ui, &pal, "Prompt Engineer", "Nicholas Slaughter");

                                ui.separator();
                                widgets::heading(ui, "Toolkits");
                                for (name, used_for, url) in TOOLKITS {
                                    widgets::link(ui, log, name, url);
                                    widgets::note(ui, &pal, used_for);
                                    ui.add_space(4.0);
                                }

                                ui.separator();
                                widgets::heading(ui, "Built with");
                                for (name, url) in LIBRARIES {
                                    widgets::link(ui, log, name, url);
                                }

                                ui.separator();
                                widgets::heading(ui, "Source code");
                                widgets::link(
                                    ui,
                                    log,
                                    "github.com/RealAnxiousGit/RustCleanup",
                                    REPO,
                                );
                                widgets::note(ui, &pal, "MIT licensed.");
                                ui.add_space(12.0);
                                if widgets::button(ui, "Close", true).clicked() {
                                    close = true;
                                }
                            });
                    });
            },
        );
        if close {
            self.open = false;
        }
    }
}
