//! Clean RAM section: Physical Memory, Virtual Memory and the Clean RAM action.
//! It is the screen's focal point, so it is the only raised panel.

use std::time::{Duration, Instant};

use eframe::egui::{self, CornerRadius, Margin, RichText, Stroke, Ui};

use super::theme::{Palette, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{n_, os, tr, trf};
use crate::settings;
use crate::sys::format::fmt_bytes;
use crate::sys::memory::{self, MemorySnapshot};

/// Usage bars turn yellow above the first share and red above the second.
const HIGH_USAGE: f32 = 0.75;
const VERY_HIGH_USAGE: f32 = 0.90;

pub struct MemoryPanel {
    snap: MemorySnapshot,
    read_at: Instant,
}

impl MemoryPanel {
    pub fn new() -> Self {
        Self {
            snap: memory::snapshot(),
            read_at: Instant::now(),
        }
    }

    /// Re-reads the figures at most once a second.
    pub fn refresh(&mut self, force: bool) {
        if force || self.read_at.elapsed() >= Duration::from_secs(1) {
            self.snap = memory::snapshot();
            self.read_at = Instant::now();
        }
    }

    pub fn start_clean(jobs: &Jobs) {
        if jobs.is_busy(Job::CleanRam) {
            return;
        }
        let purge = settings::get().purge_standby;
        jobs.log.info(tr("Cleaning RAM."));
        jobs.start(Job::CleanRam, move |log| memory::clean_ram(log, purge));
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &Jobs) {
        egui::Frame::new()
            .fill(pal.raised)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::heading(ui, tr("Clean RAM"));
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
                        if busy {
                            tr("Cleaning")
                        } else {
                            tr("Clean RAM")
                        },
                        !busy,
                    )
                    .on_hover_text(tr("Free memory other programs are holding but not using"))
                    .clicked()
                    {
                        Self::start_clean(jobs);
                    }
                    if busy {
                        ui.add(egui::Spinner::new().color(pal.muted));
                    }
                });
                ui.add_space(4.0);
                let mut purge = settings::get().purge_standby;
                if ui
                    .checkbox(
                        &mut purge,
                        tr(os(
                            n_("Also purge the standby list (file cache)"),
                            n_("Also drop the page cache (file cache, needs root)"),
                        )),
                    )
                    .on_hover_text(tr("Frees more, but recently used files load a little slower the next time"))
                    .changed()
                {
                    settings::update(|s| s.purge_standby = purge);
                }
            });
    }

    /// "8.6 GB  in use of 15.7 GB" with the percentage on the right.
    fn physical(&self, ui: &mut Ui, pal: &Palette) {
        ui.label(RichText::new(tr("Physical Memory")).family(semibold()));
        let Some(p) = self.snap.physical else {
            widgets::note(
                ui,
                pal,
                tr(os(
                    n_("Windows did not report physical memory figures. They will appear here once it does."),
                    n_("Linux did not report physical memory figures. They will appear here once it does."),
                )),
            );
            return;
        };
        let fraction = p.fraction();
        widgets::figure_row(
            ui,
            pal,
            &fmt_bytes(p.in_use()),
            &trf("in use of {0}", &[&fmt_bytes(p.usable)]),
            Some(fraction),
        );
        widgets::meter(
            ui,
            pal,
            fraction,
            widgets::usage_level(fraction, HIGH_USAGE, VERY_HIGH_USAGE),
        );
        let detail = match p.installed {
            Some(installed) => trf(
                "{0} available, {1} cached, {2} installed",
                &[
                    &fmt_bytes(p.available),
                    &fmt_bytes(p.cached),
                    &fmt_bytes(installed),
                ],
            ),
            None => trf(
                "{0} available, {1} cached",
                &[&fmt_bytes(p.available), &fmt_bytes(p.cached)],
            ),
        };
        widgets::note(ui, pal, detail);
    }

    fn virtual_memory(&self, ui: &mut Ui, pal: &Palette) {
        ui.label(RichText::new(tr("Virtual Memory")).family(semibold()));
        let Some(v) = &self.snap.virtual_mem else {
            widgets::note(
                ui,
                pal,
                tr(os(
                    n_("Windows did not report virtual memory figures."),
                    n_("Linux did not report swap figures."),
                )),
            );
            return;
        };
        if v.page_files.is_empty() {
            widgets::note(
                ui,
                pal,
                tr(os(
                    n_("No page file is configured, so Windows is running on physical memory only. You can add one in System Properties, Advanced, Performance settings."),
                    n_("No swap is configured, so Linux is running on physical memory only. You can add a swap file or zram with your distribution's tools."),
                )),
            );
        } else {
            let (used, size) = (v.page_file_in_use(), v.page_file_size());
            let fraction = used as f32 / size.max(1) as f32;
            widgets::figure_row(
                ui,
                pal,
                &fmt_bytes(used),
                &trf(os(n_("of {0} page file in use"), n_("of {0} swap in use")), &[&fmt_bytes(size)]),
                Some(fraction),
            );
            widgets::meter(
            ui,
            pal,
            fraction,
            widgets::usage_level(fraction, HIGH_USAGE, VERY_HIGH_USAGE),
        );
            let files: Vec<String> = v
                .page_files
                .iter()
                .map(|f| trf("{0} (peak {1})", &[&f.path, &fmt_bytes(f.peak)]))
                .collect();
            widgets::note(ui, pal, files.join(", "));
        }
        if v.commit_limit > 0 {
            widgets::note(
                ui,
                pal,
                trf(
                    os(n_("Committed {0} of {1} (RAM plus page file)"), n_("Committed {0} of {1} (RAM plus swap)")),
                    &[&fmt_bytes(v.committed), &fmt_bytes(v.commit_limit)],
                ),
            );
        }
    }
}
