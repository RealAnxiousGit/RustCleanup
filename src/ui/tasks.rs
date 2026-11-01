//! Running programs section and its detached task-list window.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Layout, Margin, RichText, Ui, ViewportBuilder, ViewportCommand, ViewportId,
};

use super::theme::{Palette, Theme, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::sys::format::fmt_bytes;
use crate::sys::memory;
use crate::sys::process_actions;
use crate::sys::process_monitor::{ProcInfo, ProcKind, ProcessMonitor};

/// A process at or above either of these is highlighted as resource-heavy.
const HEAVY_MEMORY: u64 = 1024 * 1024 * 1024;
const HEAVY_CPU: f32 = 15.0;
const ROW_HEIGHT: f32 = 26.0;
/// Space between task-list columns.
const COL_GAP: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortBy {
    Name,
    Cpu,
    Memory,
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Optimize,
    Suspend,
    Resume,
    End,
}

impl Action {
    fn verb(self) -> &'static str {
        match self {
            Action::Optimize => "Optimized",
            Action::Suspend => "Suspended",
            Action::Resume => "Resumed",
            Action::End => "Ended",
        }
    }
}

/// Sorted and filtered row order, rebuilt only when the sample, filter or
/// sort changes instead of every frame.
#[derive(Default)]
struct RowCache {
    key: Option<(u64, String, SortBy, bool)>,
    rows: Arc<Vec<ProcInfo>>,
    order: Vec<usize>,
}

struct TaskWindow {
    monitor: ProcessMonitor,
    filter: String,
    show_background: bool,
    selected: HashSet<u32>,
    sort: SortBy,
    cache: RowCache,
    confirm_end: Option<Vec<(u32, String)>>,
}

pub struct TaskList {
    window: Option<TaskWindow>,
    /// PIDs RustCleanup suspended, so they can be shown and resumed on exit.
    pub suspended: HashSet<u32>,
}

impl TaskList {
    pub fn new() -> Self {
        Self {
            window: None,
            suspended: HashSet::new(),
        }
    }

    pub fn open(&mut self, ctx: &egui::Context) {
        if self.window.is_some() {
            ctx.send_viewport_cmd_to(
                ViewportId::from_hash_of("task_window"),
                ViewportCommand::Focus,
            );
            return;
        }
        let repaint = ctx.clone();
        self.window = Some(TaskWindow {
            monitor: ProcessMonitor::start(move || repaint.request_repaint()),
            filter: String::new(),
            show_background: true,
            selected: HashSet::new(),
            sort: SortBy::Memory,
            cache: RowCache::default(),
            confirm_end: None,
        });
    }

    /// Resumes everything RustCleanup suspended. Called on exit.
    pub fn resume_all(&mut self) {
        for pid in self.suspended.drain() {
            let _ = process_actions::resume(pid);
        }
    }

    /// The section on the main window.
    pub fn section_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        widgets::heading(ui, "Optimize Task");
        widgets::note(
            ui,
            pal,
            "See what is using CPU and memory, then trim, slow down, pause or end it.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, "Open task list", true).clicked() {
                self.open(ui.ctx());
            }
            if !self.suspended.is_empty() {
                ui.label(
                    RichText::new(format!("{} suspended by RustCleanup", self.suspended.len()))
                        .color(pal.warn),
                );
            }
        });
    }

    /// The task list in its own native window.
    pub fn window(&mut self, ctx: &egui::Context, theme: &Theme, jobs: &mut Jobs) {
        let Some(tw) = &mut self.window else { return };
        let pal = theme.palette;
        let mut close = false;
        let mut action: Option<Action> = None;
        let mut confirmed_end: Option<Vec<(u32, String)>> = None;

        let snap = tw.monitor.latest();
        let key = (
            snap.generation,
            tw.filter.to_lowercase(),
            tw.sort,
            tw.show_background,
        );
        if tw.cache.key.as_ref() != Some(&key) {
            let rows = snap.rows.clone();
            let mut order: Vec<usize> = (0..rows.len())
                .filter(|&i| tw.show_background || rows[i].kind == ProcKind::App)
                .filter(|&i| {
                    key.1.is_empty()
                        || rows[i].name_lower.contains(&key.1)
                        || rows[i].pid.to_string() == key.1
                })
                .collect();
            match tw.sort {
                SortBy::Name => order.sort_by(|&a, &b| rows[a].name_lower.cmp(&rows[b].name_lower)),
                SortBy::Cpu => order.sort_by(|&a, &b| rows[b].cpu.total_cmp(&rows[a].cpu)),
                SortBy::Memory => order.sort_by_key(|&i| std::cmp::Reverse(rows[i].memory)),
            }
            // Forget selections and suspensions for processes that have exited.
            if snap.generation > 0 {
                let alive: HashSet<u32> = rows.iter().map(|p| p.pid).collect();
                tw.selected.retain(|pid| alive.contains(pid));
                self.suspended.retain(|pid| alive.contains(pid));
            }
            tw.cache = RowCache {
                key: Some(key),
                rows,
                order,
            };
        }

        let builder = ViewportBuilder::default()
            .with_title("Task list")
            .with_inner_size([880.0, 580.0])
            .with_min_inner_size([620.0, 340.0])
            .with_icon(super::app_icon());

        let suspended = &self.suspended;
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("task_window"),
            builder,
            |ui, _class| {
                let escape = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
                if ui.ctx().input(|i| i.viewport().close_requested())
                    || (escape && tw.confirm_end.is_none())
                {
                    close = true;
                }

                egui::Panel::top("task_tools")
                    .frame(
                        egui::Frame::new()
                            .fill(pal.bg)
                            .inner_margin(Margin::symmetric(14, 10)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Filter");
                            ui.add(
                                egui::TextEdit::singleline(&mut tw.filter)
                                    .hint_text("Name or PID")
                                    .desired_width(220.0),
                            );
                            ui.checkbox(&mut tw.show_background, "Show background processes");
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if snap.generation > 0 {
                                    widgets::note(
                                        ui,
                                        &pal,
                                        format!(
                                            "{} of {} shown",
                                            tw.cache.order.len(),
                                            snap.rows.len()
                                        ),
                                    );
                                }
                            });
                        });
                        ui.add_space(4.0);
                        let any = !tw.selected.is_empty();
                        ui.horizontal(|ui| {
                            if widgets::primary_button(ui, &pal, "Optimize", any)
                                .on_hover_text("Trim memory and set Below Normal priority")
                                .clicked()
                            {
                                action = Some(Action::Optimize);
                            }
                            if widgets::button(ui, "Suspend", any).clicked() {
                                action = Some(Action::Suspend);
                            }
                            if widgets::button(ui, "Resume", any).clicked() {
                                action = Some(Action::Resume);
                            }
                            if widgets::button(ui, "End task", any).clicked() {
                                action = Some(Action::End);
                            }
                            if widgets::button(ui, "Clear selection", any).clicked() {
                                tw.selected.clear();
                            }
                            widgets::note(ui, &pal, format!("{} selected", tw.selected.len()));
                        });
                    });

                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
                        left: 14,
                        right: 14,
                        top: 0,
                        bottom: 10,
                    }))
                    .show(ui, |ui| {
                        if let Some(err) = &snap.error {
                            ui.label(RichText::new(err).color(pal.error));
                            widgets::note(ui, &pal, "Close and reopen the task list to try again.");
                            return;
                        }
                        if snap.generation == 0 {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                                ui.label("Reading the process list.");
                            });
                            return;
                        }
                        header_row(ui, &mut tw.sort);
                        ui.separator();
                        if tw.cache.order.is_empty() {
                            ui.label(format!("No process matches \"{}\".", tw.filter));
                            if widgets::button(ui, "Clear the filter", true).clicked() {
                                tw.filter.clear();
                            }
                            return;
                        }
                        let rows = tw.cache.rows.clone();
                        let order = &tw.cache.order;
                        // Only the visible rows are laid out, however many processes there are.
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show_rows(ui, ROW_HEIGHT, order.len(), |ui, range| {
                                for &i in &order[range] {
                                    process_row(ui, &pal, &rows[i], &mut tw.selected, suspended);
                                }
                            });
                    });

                if let Some(targets) = &tw.confirm_end {
                    let mut decision = None;
                    egui::Modal::new(egui::Id::unique("confirm_end")).show(ui.ctx(), |ui| {
                        ui.set_width(380.0);
                        ui.label(
                            RichText::new("End these tasks?")
                                .family(semibold())
                                .size(16.0),
                        );
                        ui.label("Anything unsaved in them will be lost.");
                        ui.add_space(6.0);
                        for (pid, name) in targets.iter().take(12) {
                            ui.label(format!("{name} (PID {pid})"));
                        }
                        if targets.len() > 12 {
                            ui.label(format!("and {} more", targets.len() - 12));
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let end =
                                egui::Button::new(RichText::new("End task").color(pal.on_accent))
                                    .fill(pal.error);
                            if ui.add(end).clicked() {
                                decision = Some(true);
                            }
                            if widgets::button(ui, "Cancel", true).clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Escape))
                            {
                                decision = Some(false);
                            }
                        });
                    });
                    match decision {
                        Some(true) => confirmed_end = tw.confirm_end.take(),
                        Some(false) => tw.confirm_end = None,
                        None => {}
                    }
                }
            },
        );

        if let Some(action) = action {
            let targets: Vec<(u32, String, bool)> = tw
                .cache
                .rows
                .iter()
                .filter(|p| tw.selected.contains(&p.pid))
                .map(|p| (p.pid, p.name.clone(), p.protected))
                .collect();
            if let Action::End = action {
                let allowed: Vec<(u32, String)> = targets
                    .iter()
                    .filter(|t| !t.2)
                    .map(|t| (t.0, t.1.clone()))
                    .collect();
                if allowed.len() < targets.len() {
                    jobs.log.warn("Protected system processes were left out.");
                }
                if !allowed.is_empty() {
                    tw.confirm_end = Some(allowed);
                }
            } else {
                self.run(action, targets, jobs);
            }
        }
        if let Some(targets) = confirmed_end {
            if let Some(tw) = &mut self.window {
                tw.selected.clear();
            }
            self.run(
                Action::End,
                targets.into_iter().map(|(p, n)| (p, n, false)).collect(),
                jobs,
            );
        }
        if close {
            // Dropping the window also stops its sampling thread.
            self.window = None;
        }
    }

    /// Applies an action to the selected processes on a worker thread.
    fn run(&mut self, action: Action, targets: Vec<(u32, String, bool)>, jobs: &mut Jobs) {
        if jobs.is_busy(Job::ProcessAction) {
            jobs.log
                .warn("The last action is still running. Try again in a moment.");
            return;
        }
        let mut work = Vec::new();
        for (pid, name, protected) in targets {
            // Only memory trimming is safe on protected processes.
            if protected && !matches!(action, Action::Optimize) {
                jobs.log.warn(format!(
                    "{name} is a protected system process and was left alone."
                ));
                continue;
            }
            match action {
                Action::Suspend => {
                    self.suspended.insert(pid);
                }
                Action::Resume | Action::End => {
                    self.suspended.remove(&pid);
                }
                Action::Optimize => {}
            }
            work.push((pid, name, protected));
        }
        if work.is_empty() {
            return;
        }
        jobs.start(Job::ProcessAction, move |log| {
            let mut ok = 0;
            for (pid, name, protected) in &work {
                let result = match action {
                    Action::Optimize if *protected => memory::trim_process(*pid),
                    Action::Optimize => process_actions::optimize(*pid),
                    Action::Suspend => process_actions::suspend(*pid),
                    Action::Resume => process_actions::resume(*pid),
                    Action::End => process_actions::terminate(*pid),
                };
                match result {
                    Ok(()) => ok += 1,
                    Err(e) => log.warn(format!("{name} (PID {pid}): {e}")),
                }
            }
            log.success(format!(
                "{} {ok} of {} processes.",
                action.verb(),
                work.len()
            ));
            Ok(())
        });
    }
}

/// Column widths: checkbox, name (fills), PID, type, CPU, memory, status.
const COLS: [f32; 6] = [26.0, 70.0, 96.0, 70.0, 92.0, 110.0];

fn name_width(ui: &Ui) -> f32 {
    (ui.available_width() - COLS.iter().sum::<f32>() - 6.0 * COL_GAP).max(120.0)
}

fn cell<R>(ui: &mut Ui, width: f32, right: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let layout = if right {
        Layout::right_to_left(Align::Center)
    } else {
        Layout::left_to_right(Align::Center)
    };
    // A fixed rect (not the content's width) keeps every row on the same columns.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add(&mut child)
}

fn header_row(ui: &mut Ui, sort: &mut SortBy) {
    let name_w = name_width(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = COL_GAP;
        let mut sortable = |ui: &mut Ui, text: &str, key: SortBy, width: f32, right: bool| {
            cell(ui, width, right, |ui| {
                let label = if *sort == key {
                    format!("{text} \u{2193}")
                } else {
                    text.to_string()
                };
                let response = ui
                    .add(egui::Button::new(RichText::new(label).family(semibold())).frame(false))
                    .on_hover_text(format!("Sort by {}", text.to_lowercase()));
                if response.clicked() {
                    *sort = key;
                }
            });
        };
        cell(ui, COLS[0], false, |_| {});
        sortable(ui, "Name", SortBy::Name, name_w, false);
        cell(ui, COLS[1], true, |ui| {
            ui.label(RichText::new("PID").family(semibold()))
        });
        cell(ui, COLS[2], false, |ui| {
            ui.label(RichText::new("Type").family(semibold()))
        });
        sortable(ui, "CPU", SortBy::Cpu, COLS[3], true);
        sortable(ui, "Memory", SortBy::Memory, COLS[4], true);
        cell(ui, COLS[5], false, |ui| {
            ui.label(RichText::new("Status").family(semibold()))
        });
    });
}

fn process_row(
    ui: &mut Ui,
    pal: &Palette,
    p: &ProcInfo,
    selected: &mut HashSet<u32>,
    suspended: &HashSet<u32>,
) {
    let name_w = name_width(ui);
    let heavy_mem = p.memory >= HEAVY_MEMORY;
    let heavy_cpu = p.cpu >= HEAVY_CPU;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = COL_GAP;
        let mut checked = selected.contains(&p.pid);
        cell(ui, COLS[0], false, |ui| {
            if ui
                .checkbox(&mut checked, "")
                .on_hover_text(format!("Select {}", p.name))
                .changed()
            {
                if checked {
                    selected.insert(p.pid)
                } else {
                    selected.remove(&p.pid)
                };
            }
        });
        cell(ui, name_w, false, |ui| {
            let mut name = RichText::new(&p.name);
            if heavy_mem || heavy_cpu {
                name = name.family(semibold());
            }
            if ui
                .add(
                    egui::Label::new(name)
                        .truncate()
                        .sense(egui::Sense::click()),
                )
                .clicked()
            {
                if checked {
                    selected.remove(&p.pid)
                } else {
                    selected.insert(p.pid)
                };
            }
        });
        cell(ui, COLS[1], true, |ui| {
            ui.label(
                RichText::new(p.pid.to_string())
                    .monospace()
                    .color(pal.muted),
            )
        });
        cell(ui, COLS[2], false, |ui| {
            ui.label(match p.kind {
                ProcKind::App => "App",
                ProcKind::Background => "Background",
            })
        });
        cell(ui, COLS[3], true, |ui| {
            let t = RichText::new(format!("{:.1}%", p.cpu));
            ui.label(if heavy_cpu { t.color(pal.warn) } else { t })
        });
        cell(ui, COLS[4], true, |ui| {
            let t = RichText::new(fmt_bytes(p.memory));
            ui.label(if heavy_mem { t.color(pal.warn) } else { t })
        });
        cell(ui, COLS[5], false, |ui| {
            if suspended.contains(&p.pid) {
                ui.label(RichText::new("Suspended").color(pal.warn));
            } else if p.protected {
                ui.label(RichText::new("Protected").color(pal.muted))
                    .on_hover_text("System process: can only be trimmed");
            }
        });
    });
}
