//! The Activity log at the bottom of the main window.

use eframe::egui::{self, Align, Layout, RichText, Ui};

use super::theme::{Palette, semibold};
use super::widgets;
use crate::events::{Level, LogEntry};
use crate::i18n::tr;

const MAX_ENTRIES: usize = 500;

pub struct ActivityLog {
    entries: Vec<LogEntry>,
}

impl ActivityLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.drain(..self.entries.len() - MAX_ENTRIES);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Activity")).family(semibold()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let any = !self.entries.is_empty();
                if widgets::button(ui, tr("Clear"), any).clicked() {
                    self.entries.clear();
                }
                if widgets::button(ui, tr("Copy"), any).clicked() {
                    let text: String = self
                        .entries
                        .iter()
                        .map(|e| format!("[{}] {:?}: {}\n", e.time, e.level, e.message))
                        .collect();
                    ui.ctx().copy_text(text);
                }
            });
        });
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.entries.is_empty() {
                    widgets::note(
                        ui,
                        pal,
                        tr("Nothing has run yet. Everything RustCleanup changes is listed here."),
                    );
                }
                for entry in &self.entries {
                    let (tag, color) = match entry.level {
                        Level::Info => (tr("Info"), pal.muted),
                        Level::Success => (tr("Done"), pal.ok),
                        Level::Warn => (tr("Warning"), pal.warn),
                        Level::Error => (tr("Failed"), pal.error),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&entry.time).monospace().color(pal.muted));
                        ui.label(RichText::new(tag).color(color));
                        ui.label(&entry.message);
                    });
                }
            });
    }
}
