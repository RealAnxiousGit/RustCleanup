//! Running programs section and its detached task-list window.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Align, Layout, Margin, RichText, Ui};

use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{tr, trf};
use crate::sys::format::fmt_bytes;
use crate::sys::memory;
use crate::sys::process_actions;
use crate::sys::process_monitor::{ProcInfo, ProcKind, ProcessMonitor};

pub const WINDOW: &str = "task_window";
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
    /// "Optimized 3 of 4 processes." and friends.
    fn summary(self, ok: usize, total: usize) -> String {
        // One whole sentence per action, so each translates naturally.
        let args: [&dyn std::fmt::Display; 2] = [&ok, &total];
        match self {
            Action::Optimize => trf("Optimized {0} of {1} processes.", &args),
            Action::Suspend => trf("Suspended {0} of {1} processes.", &args),
            Action::Resume => trf("Resumed {0} of {1} processes.", &args),
            Action::End => trf("Ended {0} of {1} processes.", &args),
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

#[derive(Default)]
struct State {
    window: Option<TaskWindow>,
    /// PIDs RustCleanup suspended, so they can be shown and resumed on exit.
    suspended: HashSet<u32>,
}

pub struct TaskList {
    state: Arc<Mutex<State>>,
    jobs: Jobs,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl TaskList {
    pub fn new(jobs: Jobs) -> Self {
        Self {
            state: Arc::default(),
            jobs,
        }
    }

    pub fn open(&self, ctx: &egui::Context) {
        let mut state = lock(&self.state);
        if state.window.is_some() {
            super::focus_window(ctx, WINDOW);
            return;
        }
        let repaint = ctx.clone();
        let id = super::window_id(WINDOW);
        state.window = Some(TaskWindow {
            monitor: ProcessMonitor::start(move || repaint.request_repaint_of(id)),
            filter: String::new(),
            show_background: true,
            selected: HashSet::new(),
            sort: SortBy::Memory,
            cache: RowCache::default(),
            confirm_end: None,
        });
    }

    /// Resumes everything RustCleanup suspended. Called on exit.
    pub fn resume_all(&self) {
        for pid in lock(&self.state).suspended.drain() {
            let _ = process_actions::resume(pid);
        }
    }

    /// The section on the main window.
    pub fn section_ui(&self, ui: &mut Ui, pal: &Palette) {
        widgets::heading(ui, tr("Optimize Task"));
        widgets::note(
            ui,
            pal,
            tr("See what is using CPU and memory, then trim, slow down, pause or end it."),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Open task list"), true).clicked() {
                self.open(ui.ctx());
            }
            let suspended = lock(&self.state).suspended.len();
            if suspended > 0 {
                ui.label(
                    RichText::new(trf("{0} suspended by RustCleanup", &[&suspended]))
                        .color(pal.warn),
                );
            }
        });
    }

    /// Keeps the task list window alive. Called on every pass of the main window.
    pub fn show(&self, ctx: &egui::Context) {
        if lock(&self.state).window.is_none() {
            return;
        }
        let state = self.state.clone();
        let jobs = self.jobs.clone();
        ctx.show_viewport_deferred(
            super::window_id(WINDOW),
            super::window_builder(tr("Task list"), [880.0, 580.0], [620.0, 340.0]),
            move |ui, _class| {
                let mut state = lock(&state);
                window_ui(ui, &mut state, &jobs);
                if state.window.is_none() {
                    // Dropping the window also stopped its sampling thread.
                    super::closed(ui.ctx());
                }
            },
        );
    }
}

/// One pass of the task list window.
fn window_ui(ui: &mut Ui, state: &mut State, jobs: &Jobs) {
    let State { window, suspended } = state;
    let Some(tw) = window else { return };
    let pal = theme::palette();
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
            suspended.retain(|pid| alive.contains(pid));
        }
        tw.cache = RowCache {
            key: Some(key),
            rows,
            order,
        };
    }

    let escape = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
    if ui.ctx().input(|i| i.viewport().close_requested()) || (escape && tw.confirm_end.is_none()) {
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
                ui.label(tr("Filter"));
                ui.add(
                    egui::TextEdit::singleline(&mut tw.filter)
                        .hint_text(tr("Name or PID"))
                        .desired_width(220.0),
                );
                ui.checkbox(&mut tw.show_background, tr("Show background processes"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if snap.generation > 0 {
                        widgets::note(
                            ui,
                            &pal,
                            trf(
                                "{0} of {1} shown",
                                &[&tw.cache.order.len(), &snap.rows.len()],
                            ),
                        );
                    }
                });
            });
            ui.add_space(4.0);
            let any = !tw.selected.is_empty();
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, &pal, tr("Optimize"), any)
                    .on_hover_text(tr("Trim memory and set Below Normal priority"))
                    .clicked()
                {
                    action = Some(Action::Optimize);
                }
                if widgets::button(ui, tr("Suspend"), any).clicked() {
                    action = Some(Action::Suspend);
                }
                if widgets::button(ui, tr("Resume"), any).clicked() {
                    action = Some(Action::Resume);
                }
                if widgets::button(ui, tr("End task"), any).clicked() {
                    action = Some(Action::End);
                }
                if widgets::button(ui, tr("Clear selection"), any).clicked() {
                    tw.selected.clear();
                }
                widgets::note(ui, &pal, trf("{0} selected", &[&tw.selected.len()]));
                if jobs.is_busy(Job::ProcessAction) {
                    ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                }
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
                widgets::note(ui, &pal, tr("Close and reopen the task list to try again."));
                return;
            }
            if snap.generation == 0 {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                    ui.label(tr("Reading the process list."));
                });
                return;
            }
            header_row(ui, &mut tw.sort);
            ui.separator();
            if tw.cache.order.is_empty() {
                ui.label(trf("No process matches \"{0}\".", &[&tw.filter]));
                if widgets::button(ui, tr("Clear the filter"), true).clicked() {
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
                RichText::new(tr("End these tasks?"))
                    .family(semibold())
                    .size(16.0),
            );
            ui.label(tr("Anything unsaved in them will be lost."));
            ui.add_space(6.0);
            for (pid, name) in targets.iter().take(12) {
                ui.label(trf("{0} (PID {1})", &[name, pid]));
            }
            if targets.len() > 12 {
                ui.label(trf("and {0} more", &[&(targets.len() - 12)]));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let end = egui::Button::new(RichText::new(tr("End task")).color(pal.on_accent))
                    .fill(pal.error);
                if ui.add(end).clicked() {
                    decision = Some(true);
                }
                if widgets::button(ui, tr("Cancel"), true).clicked()
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
                jobs.log
                    .warn(tr("Protected system processes were left out."));
            }
            if !allowed.is_empty() {
                tw.confirm_end = Some(allowed);
            }
        } else {
            run(action, targets, suspended, jobs);
        }
    }
    if let Some(targets) = confirmed_end {
        tw.selected.clear();
        run(
            Action::End,
            targets.into_iter().map(|(p, n)| (p, n, false)).collect(),
            suspended,
            jobs,
        );
    }
    if close {
        *window = None;
    }
}

/// Applies an action to the selected processes on a worker thread.
fn run(
    action: Action,
    targets: Vec<(u32, String, bool)>,
    suspended: &mut HashSet<u32>,
    jobs: &Jobs,
) {
    if jobs.is_busy(Job::ProcessAction) {
        jobs.log.warn(tr(
            "The last action is still running. Try again in a moment.",
        ));
        return;
    }
    let mut work = Vec::new();
    for (pid, name, protected) in targets {
        // Only memory trimming is safe on protected processes.
        if protected && !matches!(action, Action::Optimize) {
            jobs.log.warn(trf(
                "{0} is a protected system process and was left alone.",
                &[&name],
            ));
            continue;
        }
        match action {
            Action::Suspend => {
                suspended.insert(pid);
            }
            Action::Resume | Action::End => {
                suspended.remove(&pid);
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
        log.success(action.summary(ok, work.len()));
        Ok(())
    });
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
        let mut sortable =
            |ui: &mut Ui, text: &str, tip: &str, key: SortBy, width: f32, right: bool| {
                cell(ui, width, right, |ui| {
                    let label = if *sort == key {
                        format!("{text} \u{2193}")
                    } else {
                        text.to_string()
                    };
                    let response = ui
                        .add(
                            egui::Button::new(RichText::new(label).family(semibold())).frame(false),
                        )
                        .on_hover_text(tip);
                    if response.clicked() {
                        *sort = key;
                    }
                });
            };
        cell(ui, COLS[0], false, |_| {});
        sortable(
            ui,
            tr("Name"),
            tr("Sort by name"),
            SortBy::Name,
            name_w,
            false,
        );
        cell(ui, COLS[1], true, |ui| {
            ui.label(RichText::new("PID").family(semibold()))
        });
        cell(ui, COLS[2], false, |ui| {
            ui.label(RichText::new(tr("Type")).family(semibold()))
        });
        sortable(ui, "CPU", tr("Sort by CPU"), SortBy::Cpu, COLS[3], true);
        sortable(
            ui,
            tr("Memory"),
            tr("Sort by memory"),
            SortBy::Memory,
            COLS[4],
            true,
        );
        cell(ui, COLS[5], false, |ui| {
            ui.label(RichText::new(tr("Status")).family(semibold()))
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
                .on_hover_text(trf("Select {0}", &[&p.name]))
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
                ProcKind::App => tr("App"),
                ProcKind::Background => tr("Background"),
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
                ui.label(RichText::new(tr("Suspended")).color(pal.warn));
            } else if p.protected {
                ui.label(RichText::new(tr("Protected")).color(pal.muted))
                    .on_hover_text(tr("System process: can only be trimmed"));
            }
        });
    });
}
