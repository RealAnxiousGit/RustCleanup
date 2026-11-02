//! Disk check section and its detached chkdsk output window.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use eframe::egui::{self, CornerRadius, Margin, RichText, Ui, ViewportId};

use super::theme::{self, Palette};
use super::widgets;
use crate::events::{Jobs, Logger};
use crate::i18n::{tr, trf};
use crate::sys::chkdsk::{self, ScanEvent, ScanMode};
use crate::sys::disk::{self, Drive, Space};
use crate::sys::format::fmt_bytes;
use crate::sys::{process_actions, shell, storage_page};

pub const WINDOW: &str = "scan_window";
const MAX_SCAN_LINES: usize = 10_000;
/// How often the free space figure is read again while the panel is shown.
const SPACE_REFRESH: Duration = Duration::from_secs(5);
const LOW_SPACE: f32 = 0.9;

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

type Shared = Arc<Mutex<Option<ScanSession>>>;

fn lock(scan: &Shared) -> MutexGuard<'_, Option<ScanSession>> {
    scan.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct DiskPanel {
    drives: Vec<Drive>,
    drive_idx: usize,
    mode: ScanMode,
    /// Shared with the output window, which drains it too.
    scan: Shared,
    jobs: Jobs,
    /// Space on the picked drive, the drive it was read for, and when.
    space: Option<(String, Option<Space>, Instant)>,
}

impl DiskPanel {
    pub fn new(jobs: Jobs) -> Self {
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
            scan: Arc::default(),
            jobs,
            space: None,
        }
    }

    fn running(&self) -> bool {
        lock(&self.scan)
            .as_ref()
            .is_some_and(|s| s.finished.is_none())
    }

    /// Stops a running chkdsk (on app exit).
    pub fn cancel(&self) {
        cancel(&lock(&self.scan), &self.jobs.log);
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette) {
        widgets::heading(ui, tr("Scan Disk"));
        widgets::note(
            ui,
            pal,
            tr("Runs chkdsk on the drive you pick and shows its output live."),
        );
        ui.add_space(6.0);

        if self.drives.is_empty() {
            widgets::note(
                ui,
                pal,
                tr("No drives were found. Connect a drive, then press Refresh."),
            );
        }
        ui.horizontal(|ui| {
            let selected = self
                .drives
                .get(self.drive_idx)
                .map(Drive::display_name)
                .unwrap_or_else(|| tr("No drives").into());
            egui::ComboBox::from_id_salt("drive")
                .width((ui.available_width() - 90.0).max(160.0))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (i, d) in self.drives.iter().enumerate() {
                        ui.selectable_value(&mut self.drive_idx, i, d.display_name());
                    }
                });
            if widgets::button(ui, tr("Refresh"), true)
                .on_hover_text(tr("Look for drives again"))
                .clicked()
            {
                self.drives = disk::list_drives();
                self.drive_idx = self.drive_idx.min(self.drives.len().saturating_sub(1));
            }
        });

        let drive = self.drives.get(self.drive_idx).cloned();
        if let Some(d) = &drive {
            ui.add_space(8.0);
            self.usage(ui, pal, d);
            ui.add_space(8.0);
        }
        // Fall back to read-only if the picked drive can't do the chosen mode.
        if let Some(d) = &drive
            && !self.mode.supports(d)
        {
            self.mode = ScanMode::ReadOnly;
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("scan_mode")
                .width((ui.available_width() - 250.0).max(160.0))
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
            if widgets::button(ui, tr("Run check"), can_run)
                .on_hover_text(tr("Run chkdsk on this drive with the mode on the left"))
                .clicked()
                && let Some(d) = &drive
            {
                self.start(ui.ctx(), d);
            }
            if widgets::button(ui, tr("Manage Storage"), drive.is_some())
                .on_hover_text(tr("Open Storage in Windows Settings for this drive"))
                .clicked()
                && let Some(d) = &drive
            {
                self.manage_storage(d);
            }
        });
        if let Some(d) = &drive {
            if !d.scannable {
                widgets::note(
                    ui,
                    pal,
                    trf("chkdsk can't check {0} drives.", &[&tr(d.kind)]),
                );
            } else if !d.is_ntfs() {
                widgets::note(
                    ui,
                    pal,
                    trf(
                        "{0} uses {1}, so only the read-only check is available.",
                        &[&d.volume(), &d.file_system],
                    ),
                );
            }
        }
        if self.running() {
            widgets::note(ui, pal, tr("A check is running in its own window."));
        }
    }

    /// Used, free and total space for the picked drive, read again every few seconds.
    fn usage(&mut self, ui: &mut Ui, pal: &Palette, drive: &Drive) {
        let stale = self
            .space
            .as_ref()
            .is_none_or(|(root, _, at)| *root != drive.root || at.elapsed() >= SPACE_REFRESH);
        if stale {
            self.space = Some((drive.root.clone(), disk::space(&drive.root), Instant::now()));
        }
        ui.ctx().request_repaint_after(SPACE_REFRESH);
        let Some((_, Some(space), _)) = &self.space else {
            widgets::note(
                ui,
                pal,
                trf(
                    "Windows did not report how much space {0} has.",
                    &[&drive.volume()],
                ),
            );
            return;
        };
        let fraction = space.fraction();
        widgets::figure_row(
            ui,
            pal,
            &fmt_bytes(space.used()),
            &trf("used of {0}", &[&fmt_bytes(space.total)]),
            Some(fraction),
        );
        widgets::meter(ui, pal, fraction, (fraction >= LOW_SPACE) as u8);
        widgets::note(
            ui,
            pal,
            trf(
                "{0} used, {1} available, {2} in total",
                &[
                    &fmt_bytes(space.used()),
                    &fmt_bytes(space.free),
                    &fmt_bytes(space.total),
                ],
            ),
        );
    }

    /// System drive: Settings > Storage. Other drives: that drive's page
    /// under Storage used on other drives, found in the background.
    fn manage_storage(&self, drive: &Drive) {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let volume = drive.volume().to_string();
        let log = self.jobs.log.clone();
        if volume.eq_ignore_ascii_case(&system) {
            match shell::open_unelevated("ms-settings:storagesense") {
                Ok(()) => log.info(trf("Opened Windows storage settings for {0}.", &[&volume])),
                Err(e) => log.error(e),
            }
            return;
        }
        // A separate thread: finding the drive's page takes a few seconds.
        std::thread::spawn(move || {
            if let Err(e) = storage_page::open_drive(&volume) {
                log.warn(trf(
                    "Settings is open at Storage, but {0} could not be selected ({1}). Open Advanced storage settings, then Storage used on other drives.",
                    &[&volume, &e],
                ));
            } else {
                log.info(trf("Opened Windows storage settings for {0}.", &[&volume]));
            }
        });
    }

    fn start(&mut self, ctx: &egui::Context, drive: &Drive) {
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        let notify = move || {
            // The main window logs the result; the output window shows the lines.
            repaint.request_repaint_of(ViewportId::ROOT);
            repaint.request_repaint_of(super::window_id(WINDOW));
        };
        match chkdsk::start_chkdsk(drive, self.mode, tx, notify) {
            Ok(handle) => {
                let title = trf("Scan Disk: {0} {1}", &[&drive.volume(), &self.mode.short()]);
                self.jobs.log.info(trf(
                    "Started chkdsk on {0} ({1}).",
                    &[&drive.volume(), &self.mode.short()],
                ));
                *lock(&self.scan) = Some(ScanSession {
                    title,
                    lines: Vec::new(),
                    progress: None,
                    rx,
                    pid: handle.pid,
                    started: Instant::now(),
                    finished: None,
                });
            }
            Err(e) => self.jobs.log.error(e),
        }
    }

    /// Drains chkdsk output. Called on every main-window pass, even while hidden.
    pub fn pump(&self) {
        if let Some(scan) = lock(&self.scan).as_mut() {
            pump(scan, &self.jobs.log);
        }
    }

    /// Keeps the chkdsk output window alive while there is a scan to show.
    pub fn show(&self, ctx: &egui::Context) {
        let Some(title) = lock(&self.scan).as_ref().map(|s| s.title.clone()) else {
            return;
        };
        let scan = self.scan.clone();
        let log = self.jobs.log.clone();
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(&title, [720.0, 460.0], [460.0, 260.0]),
            move |ui, _class| {
                let mut guard = lock(&scan);
                let Some(session) = guard.as_mut() else {
                    return;
                };
                pump(session, &log);
                if window_ui(ui, session) {
                    cancel(&guard, &log);
                    *guard = None;
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

fn cancel(scan: &Option<ScanSession>, log: &Logger) {
    if let Some(scan) = scan
        && scan.finished.is_none()
    {
        match process_actions::terminate(scan.pid) {
            Ok(()) => log.warn(tr("Disk check cancelled.")),
            Err(e) => log.error(trf("Could not stop chkdsk: {0}", &[&e])),
        }
    }
}

/// Moves new chkdsk output from the channel into the session.
fn pump(scan: &mut ScanSession, log: &Logger) {
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
                        let msg = trf(
                            "{0} finished: {1}",
                            &[&scan.title, &chkdsk::describe_exit(*code)],
                        );
                        if *code == 0 {
                            log.success(msg)
                        } else {
                            log.warn(msg)
                        }
                    }
                    Err(e) => log.error(trf("{0} failed: {1}", &[&scan.title, e])),
                }
                scan.finished = Some((result, scan.started.elapsed()));
            }
        }
    }
}

/// One pass of the output window. Returns true when it should close.
fn window_ui(ui: &mut Ui, scan: &mut ScanSession) -> bool {
    let pal = theme::palette();
    let mut close = super::close_pressed(ui.ctx());

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
                    ui.label(trf(
                        "Running for {0} s",
                        &[&scan.started.elapsed().as_secs()],
                    ));
                }
                Some((Ok(code), took)) => {
                    let color = if *code == 0 { pal.ok } else { pal.warn };
                    ui.label(
                        RichText::new(trf(
                            "Finished in {0} s. {1}",
                            &[&took.as_secs(), &chkdsk::describe_exit(*code)],
                        ))
                        .color(color),
                    );
                }
                Some((Err(e), _)) => {
                    ui.label(
                        RichText::new(trf("chkdsk could not run: {0}", &[e])).color(pal.error),
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
                    if widgets::button(ui, tr("Stop check"), true).clicked() {
                        close = true;
                    }
                } else if widgets::button(ui, tr("Close"), true).clicked() {
                    close = true;
                }
                if widgets::button(ui, tr("Copy output"), !scan.lines.is_empty()).clicked() {
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
                            let text =
                                |s: &str| RichText::new(s).monospace().color(pal.terminal_text);
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
                                ui.label(text(tr("Waiting for chkdsk to start.")));
                            }
                        });
                });
        });

    if scan.finished.is_none() {
        // Keep the elapsed-time counter ticking.
        ui.ctx().request_repaint_after(Duration::from_millis(500));
    }
    close
}
