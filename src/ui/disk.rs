//! Disk check section and its detached chkdsk output window.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, CornerRadius, Margin, RichText, Ui, ViewportBuilder, ViewportId};

use super::theme::{Palette, Theme};
use super::widgets;
use crate::events::Jobs;
use crate::sys::chkdsk::{self, ScanEvent, ScanMode};
use crate::sys::disk::{self, Drive};
use crate::sys::process_actions;

const MAX_SCAN_LINES: usize = 10_000;

/// A running or finished chkdsk session.
struct ScanSession {
    title: String,
    lines: Vec<String>,
    /// Current progress line (chkdsk redraws it in place with a carriage return).
    progress: Option<String>,
    rx: Receiver<ScanEvent>,
    pid: u32,
    started: Instant,
    finished: Option<(Result<i32, String>, Duration)>,
}

pub struct DiskPanel {
    drives: Vec<Drive>,
    drive_idx: usize,
    mode: ScanMode,
    scan: Option<ScanSession>,
}

impl DiskPanel {
    pub fn new() -> Self {
        let drives = disk::list_drives();
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let drive_idx = drives
            .iter()
            .position(|d| d.volume().eq_ignore_ascii_case(&system))
            .unwrap_or(0);
        Self {
            drives,
            drive_idx,
            mode: ScanMode::ReadOnly,
            scan: None,
        }
    }

    fn running(&self) -> bool {
        self.scan.as_ref().is_some_and(|s| s.finished.is_none())
    }

    /// Stops a running chkdsk (on app exit, or when its window closes).
    pub fn cancel(&mut self, jobs: &Jobs) {
        if let Some(scan) = &self.scan
            && scan.finished.is_none()
        {
            match process_actions::terminate(scan.pid) {
                Ok(()) => jobs.log.warn("Disk check cancelled."),
                Err(e) => jobs.log.error(format!("Could not stop chkdsk: {e}")),
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &mut Jobs) {
        widgets::heading(ui, "Scan Disk");
        widgets::note(
            ui,
            pal,
            "Runs chkdsk on the drive you pick and shows its output live.",
        );
        ui.add_space(6.0);

        if self.drives.is_empty() {
            widgets::note(
                ui,
                pal,
                "No drives were found. Connect a drive, then press Refresh.",
            );
        }
        ui.horizontal(|ui| {
            let selected = self
                .drives
                .get(self.drive_idx)
                .map(Drive::display_name)
                .unwrap_or_else(|| "No drives".into());
            egui::ComboBox::from_id_salt("drive")
                .width((ui.available_width() - 90.0).max(160.0))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (i, d) in self.drives.iter().enumerate() {
                        ui.selectable_value(&mut self.drive_idx, i, d.display_name());
                    }
                });
            if widgets::button(ui, "Refresh", true)
                .on_hover_text("Look for drives again")
                .clicked()
            {
                self.drives = disk::list_drives();
                self.drive_idx = self.drive_idx.min(self.drives.len().saturating_sub(1));
            }
        });

        let drive = self.drives.get(self.drive_idx).cloned();
        // Fall back to read-only if the picked drive can't do the chosen mode.
        if let Some(d) = &drive
            && !self.mode.supports(d)
        {
            self.mode = ScanMode::ReadOnly;
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("scan_mode")
                .width((ui.available_width() - 130.0).max(160.0))
                .selected_text(self.mode.label())
                .show_ui(ui, |ui| {
                    for mode in ScanMode::ALL {
                        let ok = drive.as_ref().is_some_and(|d| mode.supports(d));
                        ui.add_enabled_ui(ok, |ui| {
                            ui.selectable_value(&mut self.mode, mode, mode.label())
                        });
                    }
                });
            let can_run = drive.as_ref().is_some_and(|d| self.mode.supports(d)) && !self.running();
            if widgets::button(ui, "Run check", can_run).clicked()
                && let Some(d) = &drive
            {
                self.start(ui.ctx(), d, jobs);
            }
        });
        if let Some(d) = &drive {
            if !d.scannable {
                widgets::note(ui, pal, format!("chkdsk can't check {} drives.", d.kind));
            } else if !d.is_ntfs() {
                widgets::note(
                    ui,
                    pal,
                    format!(
                        "{} uses {}, so only the read-only check is available.",
                        d.volume(),
                        d.file_system
                    ),
                );
            }
        }
        if self.running() {
            widgets::note(ui, pal, "A check is running in its own window.");
        }
    }

    fn start(&mut self, ctx: &egui::Context, drive: &Drive, jobs: &Jobs) {
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        match chkdsk::start_chkdsk(drive, self.mode, tx, move || repaint.request_repaint()) {
            Ok(handle) => {
                let title = format!("Scan Disk: {} {}", drive.volume(), self.mode.short());
                jobs.log.info(format!(
                    "Started chkdsk on {} ({}).",
                    drive.volume(),
                    self.mode.short()
                ));
                self.scan = Some(ScanSession {
                    title,
                    lines: Vec::new(),
                    progress: None,
                    rx,
                    pid: handle.pid,
                    started: Instant::now(),
                    finished: None,
                });
            }
            Err(e) => jobs.log.error(e),
        }
    }

    /// Drains chkdsk output. Called every frame, even while the window is hidden.
    pub fn pump(&mut self, jobs: &Jobs) {
        let Some(scan) = &mut self.scan else { return };
        while let Ok(event) = scan.rx.try_recv() {
            match event {
                ScanEvent::Line(line) => {
                    // A finished progress line is replaced by the real line.
                    scan.progress = None;
                    scan.lines.push(line);
                    if scan.lines.len() > MAX_SCAN_LINES {
                        scan.lines.drain(..scan.lines.len() - MAX_SCAN_LINES);
                    }
                }
                ScanEvent::Progress(p) if !p.trim().is_empty() => scan.progress = Some(p),
                ScanEvent::Progress(_) => {}
                ScanEvent::Finished(result) => {
                    match &result {
                        Ok(code) => {
                            let msg = format!(
                                "{} finished: {}",
                                scan.title,
                                chkdsk::describe_exit(*code)
                            );
                            if *code == 0 {
                                jobs.log.success(msg)
                            } else {
                                jobs.log.warn(msg)
                            }
                        }
                        Err(e) => jobs.log.error(format!("{} failed: {e}", scan.title)),
                    }
                    scan.finished = Some((result, scan.started.elapsed()));
                }
            }
        }
    }

    /// The chkdsk output, in its own native window.
    pub fn window(&mut self, ctx: &egui::Context, theme: &Theme, jobs: &Jobs) {
        let Some(scan) = &mut self.scan else { return };
        let pal = theme.palette;
        let mut close = false;
        let builder = ViewportBuilder::default()
            .with_title(scan.title.clone())
            .with_inner_size([720.0, 460.0])
            .with_min_inner_size([460.0, 260.0])
            .with_icon(super::app_icon());

        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("scan_window"),
            builder,
            |ui, _class| {
                if ui
                    .ctx()
                    .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
                {
                    close = true;
                }

                egui::Panel::top("scan_status")
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::symmetric(14, 10)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| match &scan.finished {
                            None => {
                                ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                                ui.label(format!(
                                    "Running for {} s",
                                    scan.started.elapsed().as_secs()
                                ));
                            }
                            Some((Ok(code), took)) => {
                                let color = if *code == 0 { pal.ok } else { pal.warn };
                                ui.label(
                                    RichText::new(format!(
                                        "Finished in {} s. {}",
                                        took.as_secs(),
                                        chkdsk::describe_exit(*code)
                                    ))
                                    .color(color),
                                );
                            }
                            Some((Err(e), _)) => {
                                ui.label(
                                    RichText::new(format!("chkdsk could not run: {e}"))
                                        .color(pal.error),
                                );
                            }
                        });
                    });

                egui::Panel::bottom("scan_actions")
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::symmetric(14, 10)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if scan.finished.is_none() {
                                if widgets::button(ui, "Stop check", true).clicked() {
                                    close = true;
                                }
                            } else if widgets::button(ui, "Close", true).clicked() {
                                close = true;
                            }
                            if widgets::button(ui, "Copy output", !scan.lines.is_empty()).clicked()
                            {
                                ui.ctx().copy_text(scan.lines.join("\n"));
                            }
                        });
                    });

                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                        left: 14,
                        right: 14,
                        top: 0,
                        bottom: 0,
                    }))
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .fill(pal.terminal_bg)
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                // The scroll area takes exactly the space left, so the
                                // window never grows with the output.
                                egui::ScrollArea::both()
                                    .stick_to_bottom(true)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let text = |s: &str| {
                                            RichText::new(s).monospace().color(pal.terminal_text)
                                        };
                                        for line in &scan.lines {
                                            ui.label(text(line));
                                        }
                                        if let Some(p) = &scan.progress {
                                            ui.label(text(p));
                                        }
                                        if scan.lines.is_empty()
                                            && scan.progress.is_none()
                                            && scan.finished.is_none()
                                        {
                                            ui.label(text("Waiting for chkdsk to start."));
                                        }
                                    });
                            });
                    });

                if scan.finished.is_none() {
                    // Keep the elapsed-time counter ticking.
                    ui.ctx().request_repaint_after(Duration::from_millis(500));
                }
            },
        );

        if close {
            self.cancel(jobs);
            self.scan = None;
        }
    }
}
