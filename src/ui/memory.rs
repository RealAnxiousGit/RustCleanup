//! Clean RAM section: Physical Memory, Virtual Memory and the Clean RAM action.
//! It is the screen's focal point, so it is the only raised panel.

use std::time::{Duration, Instant};

use eframe::egui::{self, CornerRadius, Margin, RichText, Stroke, Ui};

use super::theme::{Palette, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::sys::format::fmt_bytes;
use crate::sys::memory::{self, MemorySnapshot};

/// Physical memory above this share of RAM is shown in the warning colour.
const HIGH_USAGE: f32 = 0.85;

pub struct MemoryPanel {
    snap: MemorySnapshot,
    read_at: Instant,
    pub purge_standby: bool,
}

impl MemoryPanel {
    pub fn new() -> Self {
        Self {
            snap: memory::snapshot(),
            read_at: Instant::now(),
            purge_standby: true,
        }
    }

    /// Re-reads the figures at most once a second.
    pub fn refresh(&mut self, force: bool) {
        if force || self.read_at.elapsed() >= Duration::from_secs(1) {
            self.snap = memory::snapshot();
            self.read_at = Instant::now();
        }
    }

    pub fn start_clean(&self, jobs: &mut Jobs) {
        let purge = self.purge_standby;
        jobs.log.info("Cleaning RAM.");
        jobs.start(Job::CleanRam, move |log| memory::clean_ram(log, purge));
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &mut Jobs) {
        egui::Frame::new()
            .fill(pal.raised)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::heading(ui, "Clean RAM");
                ui.add_space(10.0);
                self.physical(ui, pal);
                ui.add_space(16.0);
                self.virtual_memory(ui, pal);
                ui.add_space(18.0);

                let busy = jobs.is_busy(Job::CleanRam);
                ui.horizontal(|ui| {
                    if widgets::primary_button(
                        ui,
                        pal,
                        if busy { "Cleaning" } else { "Clean RAM" },
                        !busy,
                    )
                    .clicked()
                    {
                        self.start_clean(jobs);
                    }
                    if busy {
                        ui.add(egui::Spinner::new().color(pal.muted));
                    }
                });
                ui.add_space(4.0);
                ui.checkbox(
                    &mut self.purge_standby,
                    "Also purge the standby list (file cache)",
                );
            });
    }

    /// "8.6 GB  in use of 15.7 GB" with the percentage on the right.
    fn figure_row(ui: &mut Ui, pal: &Palette, big: &str, rest: &str, percent: Option<f32>) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(big)
                    .family(semibold())
                    .size(22.0)
                    .color(pal.text),
            );
            ui.label(RichText::new(rest).color(pal.muted));
            if let Some(p) = percent {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.0}%", p * 100.0))
                            .family(semibold())
                            .size(15.0),
                    );
                });
            }
        });
    }

    fn physical(&self, ui: &mut Ui, pal: &Palette) {
        ui.label(RichText::new("Physical Memory").family(semibold()));
        let Some(p) = self.snap.physical else {
            widgets::note(
                ui,
                pal,
                "Windows did not report physical memory figures. They will appear here once it does.",
            );
            return;
        };
        let fraction = p.fraction();
        Self::figure_row(
            ui,
            pal,
            &fmt_bytes(p.in_use()),
            &format!("in use of {}", fmt_bytes(p.usable)),
            Some(fraction),
        );
        widgets::meter(ui, pal, fraction, fraction >= HIGH_USAGE);
        let mut detail = format!(
            "{} available, {} cached",
            fmt_bytes(p.available),
            fmt_bytes(p.cached)
        );
        if let Some(installed) = p.installed {
            detail.push_str(&format!(", {} installed", fmt_bytes(installed)));
        }
        widgets::note(ui, pal, detail);
    }

    fn virtual_memory(&self, ui: &mut Ui, pal: &Palette) {
        ui.label(RichText::new("Virtual Memory").family(semibold()));
        let Some(v) = &self.snap.virtual_mem else {
            widgets::note(ui, pal, "Windows did not report virtual memory figures.");
            return;
        };
        if v.page_files.is_empty() {
            widgets::note(
                ui,
                pal,
                "No page file is configured, so Windows is running on physical memory only. \
                 You can add one in System Properties, Advanced, Performance settings.",
            );
        } else {
            let (used, size) = (v.page_file_in_use(), v.page_file_size());
            let fraction = used as f32 / size.max(1) as f32;
            Self::figure_row(
                ui,
                pal,
                &fmt_bytes(used),
                &format!("of {} page file in use", fmt_bytes(size)),
                Some(fraction),
            );
            widgets::meter(ui, pal, fraction, fraction >= HIGH_USAGE);
            let files: Vec<String> = v
                .page_files
                .iter()
                .map(|f| format!("{} (peak {})", f.path, fmt_bytes(f.peak)))
                .collect();
            widgets::note(ui, pal, files.join(", "));
        }
        if v.commit_limit > 0 {
            widgets::note(
                ui,
                pal,
                format!(
                    "Committed {} of {} (RAM plus page file)",
                    fmt_bytes(v.committed),
                    fmt_bytes(v.commit_limit)
                ),
            );
        }
    }
}
