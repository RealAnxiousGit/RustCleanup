//! Running programs section and its detached task-list window.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Id, Layout, Margin, Pos2, Rect, RichText,
    Sense, Shape, Stroke, TextStyle, Ui,
    text::{LayoutJob, TextFormat, TextWrapping},
};

use super::theme::{self, Palette, semibold};
use super::widgets;
use crate::events::{Job, Jobs};
use crate::i18n::{tr, trf};
use crate::sys::format::fmt_bytes;
use crate::sys::memory;
use crate::sys::process_actions;
use crate::sys::process_monitor::{ProcInfo, ProcKind, ProcessMonitor};

pub const WINDOW: &str = "task_window";
/// CPU use at or above this is shown in the warning colour.
const HEAVY_CPU: f32 = 15.0;
/// Shares of physical RAM at which a process's memory turns yellow, then red.
const MEMORY_WARN: f64 = 0.04;
const MEMORY_HEAVY: f64 = 0.10;
const ROW_HEIGHT: f32 = 28.0;
const HEADER_HEIGHT: f32 = 32.0;
/// Space between task-list columns.
const COL_GAP: f32 = 16.0;
/// Room on the left of each row for the selection bar.
const ROW_INSET: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortBy {
    Name,
    Pid,
    Type,
    Cpu,
    Memory,
    Status,
}

/// The columns, left to right: what they sort by and whether their text is
/// right-aligned (numbers are).
const COLUMNS: [(SortBy, bool); 6] = [
    (SortBy::Name, false),
    (SortBy::Pid, true),
    (SortBy::Type, false),
    (SortBy::Cpu, true),
    (SortBy::Memory, true),
    (SortBy::Status, false),
];
/// Fixed widths of every column but Name, which takes the rest.
const WIDTHS: [f32; 5] = [70.0, 104.0, 70.0, 96.0, 110.0];

impl SortBy {
    fn title(self) -> &'static str {
        match self {
            SortBy::Name => tr("Name"),
            SortBy::Pid => "PID",
            SortBy::Type => tr("Type"),
            SortBy::Cpu => "CPU",
            SortBy::Memory => tr("Memory"),
            SortBy::Status => tr("Status"),
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            SortBy::Name => tr("Sort by name"),
            SortBy::Pid => tr("Sort by process ID"),
            SortBy::Type => tr("Sort by type: apps first, then background processes"),
            SortBy::Cpu => tr("Sort by CPU"),
            SortBy::Memory => tr("Sort by memory"),
            SortBy::Status => tr("Sort by status: suspended and protected first"),
        }
    }

    /// Numbers start with the biggest, everything else from A.
    fn starts_descending(self) -> bool {
        matches!(self, SortBy::Cpu | SortBy::Memory)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Optimize,
    Suspend,
    Resume,
    Restart,
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
            Action::Restart => trf("Restarted {0} of {1} programs.", &args),
            Action::End => trf("Ended {0} of {1} processes.", &args),
        }
    }
}

/// Sorted and filtered row order, rebuilt only when the sample, filter or
/// sort changes instead of every frame.
#[derive(Default)]
struct RowCache {
    key: Option<(u64, String, SortBy, bool, bool)>,
    rows: Arc<Vec<ProcInfo>>,
    order: Vec<usize>,
}

struct TaskWindow {
    monitor: ProcessMonitor,
    filter: String,
    show_background: bool,
    selected: HashSet<u32>,
    /// Where a Shift+click range starts, like in File Explorer.
    anchor: Option<u32>,
    sort: SortBy,
    descending: bool,
    cache: RowCache,
    /// End and Restart ask first.
    confirm: Option<(Action, Vec<(u32, String)>)>,
    /// Physical RAM, to judge how heavy each process is.
    ram: u64,
}

#[derive(Default)]
struct State {
    window: Option<TaskWindow>,
    /// PIDs RustCleanup suspended, so they can be resumed on exit.
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
            anchor: None,
            sort: SortBy::Memory,
            descending: true,
            cache: RowCache::default(),
            confirm: None,
            ram: memory::snapshot()
                .physical
                .map(|p| p.usable)
                .filter(|&r| r > 0)
                .unwrap_or(16 << 30),
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
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Open task list"), true)
                .on_hover_text(tr("Every running program with its CPU and memory use"))
                .clicked()
            {
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
            super::window_builder(tr("Task list"), [920.0, 600.0], [660.0, 360.0]),
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

/// Order of two rows for a column, before the direction is applied.
fn compare(sort: SortBy, a: &ProcInfo, b: &ProcInfo) -> Ordering {
    let status = |p: &ProcInfo| {
        if p.suspended {
            0
        } else if p.protected {
            1
        } else {
            2
        }
    };
    let kind = |p: &ProcInfo| (p.kind != ProcKind::App) as u8;
    let by_name = || a.name_lower.cmp(&b.name_lower).then(a.pid.cmp(&b.pid));
    match sort {
        SortBy::Name => by_name(),
        SortBy::Pid => a.pid.cmp(&b.pid),
        SortBy::Type => kind(a).cmp(&kind(b)).then_with(by_name),
        SortBy::Cpu => a.cpu.total_cmp(&b.cpu).then_with(|| by_name().reverse()),
        SortBy::Memory => a.memory.cmp(&b.memory).then_with(|| by_name().reverse()),
        SortBy::Status => status(a).cmp(&status(b)).then_with(by_name),
    }
}

/// One pass of the task list window.
fn window_ui(ui: &mut Ui, state: &mut State, jobs: &Jobs) {
    let State { window, suspended } = state;
    let Some(tw) = window else { return };
    let pal = theme::palette();
    let mut close = false;
    let mut action: Option<Action> = None;
    let mut confirmed: Option<(Action, Vec<(u32, String)>)> = None;

    let snap = tw.monitor.latest();
    let key = (
        snap.generation,
        tw.filter.to_lowercase(),
        tw.sort,
        tw.descending,
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
        let (sort, descending) = (tw.sort, tw.descending);
        order.sort_by(|&a, &b| {
            let o = compare(sort, &rows[a], &rows[b]);
            if descending { o.reverse() } else { o }
        });
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

    let ctx = ui.ctx().clone();
    let typing = ctx.egui_wants_keyboard_input();
    let (escape, select_all, delete) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.modifiers.command && i.key_pressed(egui::Key::A),
            i.key_pressed(egui::Key::Delete),
        )
    });
    if tw.confirm.is_none() {
        if ctx.input(|i| i.viewport().close_requested()) {
            close = true;
        } else if escape && !tw.selected.is_empty() {
            tw.selected.clear();
        } else if escape {
            close = true;
        }
        if select_all && !typing {
            tw.selected = tw.cache.order.iter().map(|&i| tw.cache.rows[i].pid).collect();
        }
        if delete && !typing && !tw.selected.is_empty() {
            action = Some(Action::End);
        }
    }

    // What the selection allows, for the toolbar and the right-click menu.
    let mut can = Can::default();
    for p in tw.cache.rows.iter().filter(|p| tw.selected.contains(&p.pid)) {
        can.any = true;
        can.unprotected |= !p.protected;
        can.suspend |= !p.protected && !p.suspended;
        can.resume |= p.suspended;
    }

    egui::Panel::top("task_tools")
        .frame(
            egui::Frame::new()
                .fill(pal.bg)
                .inner_margin(Margin::symmetric(16, 12)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Filter"));
                ui.add(
                    egui::TextEdit::singleline(&mut tw.filter)
                        .hint_text(tr("Name or PID"))
                        .desired_width(220.0),
                )
                .on_hover_text(tr("Show only processes whose name contains this text, or one PID"));
                ui.add_space(6.0);
                ui.checkbox(&mut tw.show_background, tr("Show background processes"))
                    .on_hover_text(tr(
                        "Background processes have no window of their own, like services and helpers",
                    ));
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
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if let Some(a) = action_buttons(ui, &pal, can) {
                    action = Some(a);
                }
                if widgets::button(ui, tr("Clear selection"), can.any)
                    .on_hover_text(tr("Deselect everything (Esc)"))
                    .clicked()
                {
                    tw.selected.clear();
                }
                ui.add_space(4.0);
                widgets::note(ui, &pal, trf("{0} selected", &[&tw.selected.len()]));
                if jobs.is_busy(Job::ProcessAction) {
                    ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                }
            });
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(pal.bg).inner_margin(Margin {
            left: 16,
            right: 16,
            top: 0,
            bottom: 12,
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
            header_row(ui, &pal, &mut tw.sort, &mut tw.descending);
            if tw.cache.order.is_empty() {
                ui.add_space(8.0);
                ui.label(trf("No process matches \"{0}\".", &[&tw.filter]));
                if widgets::button(ui, tr("Clear the filter"), true).clicked() {
                    tw.filter.clear();
                }
                return;
            }
            let rows = tw.cache.rows.clone();
            let order = &tw.cache.order;
            let fonts = RowFonts::new(ui);
            let mut clicked: Option<(usize, egui::Modifiers)> = None;
            // Rows touch, so hover and selection fills form one block.
            ui.spacing_mut().item_spacing.y = 0.0;
            // Only the visible rows are laid out, however many processes there are.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, ROW_HEIGHT, order.len(), |ui, range| {
                    let start = range.start;
                    for (n, &i) in order[range].iter().enumerate() {
                        let p = &rows[i];
                        let response = process_row(ui, &pal, &fonts, p, tw.selected.contains(&p.pid), tw.ram);
                        if response.clicked() {
                            clicked = Some((start + n, ui.input(|i| i.modifiers)));
                        }
                        if response.secondary_clicked() && !tw.selected.contains(&p.pid) {
                            tw.selected = HashSet::from([p.pid]);
                            tw.anchor = Some(p.pid);
                        }
                        response.context_menu(|ui| {
                            if let Some(a) = context_menu(ui, can) {
                                action = Some(a);
                            }
                        });
                    }
                });
            if let Some((index, modifiers)) = clicked {
                select(tw, &rows, index, modifiers);
            }
        });

    if let Some((kind, targets)) = &tw.confirm {
        let mut decision = None;
        egui::Modal::new(Id::unique("confirm_task_action")).show(&ctx, |ui| {
            ui.set_width(400.0);
            let (title, detail, button) = match kind {
                Action::Restart => (
                    tr("Restart these tasks?"),
                    tr("They will close and open again. Anything unsaved in them will be lost."),
                    tr("Restart"),
                ),
                _ => (
                    tr("End these tasks?"),
                    tr("Anything unsaved in them will be lost."),
                    tr("End task"),
                ),
            };
            ui.label(RichText::new(title).family(semibold()).size(16.0));
            ui.add_space(2.0);
            ui.label(detail);
            ui.add_space(8.0);
            for (pid, name) in targets.iter().take(12) {
                ui.label(trf("{0} (PID {1})", &[name, pid]));
            }
            if targets.len() > 12 {
                ui.label(trf("and {0} more", &[&(targets.len() - 12)]));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if widgets::danger_button(ui, &pal, button).clicked() {
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
            Some(true) => confirmed = tw.confirm.take(),
            Some(false) => tw.confirm = None,
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
        if matches!(action, Action::End | Action::Restart) {
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
                tw.confirm = Some((action, allowed));
            }
        } else {
            run(action, targets, suspended, jobs, tw.monitor.waker());
        }
    }
    if let Some((action, targets)) = confirmed {
        tw.selected.clear();
        run(
            action,
            targets.into_iter().map(|(p, n)| (p, n, false)).collect(),
            suspended,
            jobs,
            tw.monitor.waker(),
        );
    }
    if close {
        *window = None;
    }
}

/// What the current selection allows.
#[derive(Clone, Copy, Default)]
struct Can {
    any: bool,
    unprotected: bool,
    suspend: bool,
    resume: bool,
}

fn action_buttons(ui: &mut Ui, pal: &Palette, can: Can) -> Option<Action> {
    let mut action = None;
    if widgets::primary_button(ui, pal, tr("Optimize"), can.any)
        .on_hover_text(tr("Trim memory and set Below Normal priority"))
        .clicked()
    {
        action = Some(Action::Optimize);
    }
    let buttons = [
        (Action::Suspend, tr("Suspend"), tr("Pause it completely until you resume it"), can.suspend),
        (Action::Resume, tr("Resume"), tr("Let a suspended process run again"), can.resume),
        (
            Action::Restart,
            tr("Restart"),
            tr("Close it and open it again with the same settings"),
            can.unprotected,
        ),
        (Action::End, tr("End task"), tr("Close it right away (Delete)"), can.unprotected),
    ];
    for (a, label, tip, enabled) in buttons {
        if widgets::button(ui, label, enabled)
            .on_hover_text(tip)
            .on_disabled_hover_text(tip)
            .clicked()
        {
            action = Some(a);
        }
    }
    action
}

fn context_menu(ui: &mut Ui, can: Can) -> Option<Action> {
    let items = [
        (Action::Optimize, tr("Optimize"), can.any),
        (Action::Suspend, tr("Suspend"), can.suspend),
        (Action::Resume, tr("Resume"), can.resume),
        (Action::Restart, tr("Restart"), can.unprotected),
        (Action::End, tr("End task"), can.unprotected),
    ];
    let mut action = None;
    for (a, label, enabled) in items {
        if a == Action::End {
            ui.separator();
        }
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            action = Some(a);
            ui.close();
        }
    }
    action
}

/// Click, Ctrl+click and Shift+click, the way File Explorer selects files.
fn select(tw: &mut TaskWindow, rows: &[ProcInfo], index: usize, modifiers: egui::Modifiers) {
    let order = &tw.cache.order;
    let pid = rows[order[index]].pid;
    let anchor = tw
        .anchor
        .and_then(|a| order.iter().position(|&i| rows[i].pid == a));
    match (modifiers.shift, anchor) {
        (true, Some(anchor)) => {
            let (from, to) = (anchor.min(index), anchor.max(index));
            if !modifiers.command {
                tw.selected.clear();
            }
            tw.selected
                .extend(order[from..=to].iter().map(|&i| rows[i].pid));
        }
        _ if modifiers.command => {
            if !tw.selected.remove(&pid) {
                tw.selected.insert(pid);
            }
            tw.anchor = Some(pid);
        }
        _ => {
            tw.selected = HashSet::from([pid]);
            tw.anchor = Some(pid);
        }
    }
}

/// Applies an action to the selected processes on a worker thread.
fn run(
    action: Action,
    targets: Vec<(u32, String, bool)>,
    suspended: &mut HashSet<u32>,
    jobs: &Jobs,
    refresh: Arc<AtomicBool>,
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
            Action::Resume | Action::Restart | Action::End => {
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
        if action == Action::Restart {
            let pids: Vec<u32> = work.iter().map(|w| w.0).collect();
            let results = process_actions::restart(&pids);
            let ok = results.iter().filter(|r| r.1.is_ok()).count();
            for (name, result) in &results {
                if let Err(e) = result {
                    log.warn(trf("Could not restart {0}: {1}", &[name, e]));
                }
            }
            refresh.store(true, AtomicOrdering::Relaxed);
            log.success(action.summary(ok, results.len()));
            return Ok(());
        }
        let mut ok = 0;
        for (pid, name, protected) in &work {
            let result = match action {
                Action::Optimize if *protected => memory::trim_process(*pid),
                Action::Optimize => process_actions::optimize(*pid),
                Action::Suspend => process_actions::suspend(*pid),
                Action::Resume => process_actions::resume(*pid),
                Action::End | Action::Restart => process_actions::terminate(*pid),
            };
            match result {
                Ok(()) => ok += 1,
                Err(e) => log.warn(format!("{name} (PID {pid}): {e}")),
            }
        }
        // Show the new state right away instead of at the next sample.
        refresh.store(true, AtomicOrdering::Relaxed);
        log.success(action.summary(ok, work.len()));
        Ok(())
    });
}

/// Left and right edge of each column inside `rect`.
fn column_spans(rect: Rect) -> [(f32, f32); 6] {
    let left = rect.left() + ROW_INSET;
    let fixed: f32 = WIDTHS.iter().sum::<f32>() + 5.0 * COL_GAP;
    let name_w = (rect.right() - 8.0 - left - fixed).max(120.0);
    let mut spans = [(0.0, 0.0); 6];
    let mut x = left;
    for (i, span) in spans.iter_mut().enumerate() {
        let w = if i == 0 { name_w } else { WIDTHS[i - 1] };
        *span = (x, x + w);
        x += w + COL_GAP;
    }
    spans
}

/// Text that fits in `width`, cut with "..." when it doesn't.
fn fitted(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

/// Paints text in a column, left- or right-aligned and centred vertically.
fn paint_cell(ui: &Ui, span: (f32, f32), y: f32, right: bool, galley: Arc<egui::Galley>) {
    let size = galley.size();
    let x = if right { span.1 - size.x } else { span.0 };
    let color = ui.visuals().text_color();
    ui.painter()
        .galley(Pos2::new(x, y - size.y / 2.0), galley, color);
}

/// A small sort triangle pointing up (ascending) or down.
fn sort_arrow(ui: &Ui, center: Pos2, down: bool, color: Color32) {
    let (w, h) = (4.5, 3.0);
    let points = if down {
        vec![
            center + egui::vec2(-w, -h),
            center + egui::vec2(w, -h),
            center + egui::vec2(0.0, h),
        ]
    } else {
        vec![
            center + egui::vec2(-w, h),
            center + egui::vec2(w, h),
            center + egui::vec2(0.0, -h),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// The column headers. Clicking one sorts by it; clicking it again reverses.
fn header_row(ui: &mut Ui, pal: &Palette, sort: &mut SortBy, descending: &mut bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEADER_HEIGHT),
        Sense::hover(),
    );
    let spans = column_spans(rect);
    let font = FontId::new(TextStyle::Body.resolve(ui.style()).size, semibold());
    for (i, &(key, right)) in COLUMNS.iter().enumerate() {
        let (x0, x1) = spans[i];
        let cell = Rect::from_x_y_ranges(
            (x0 - COL_GAP / 2.0)..=(x1 + COL_GAP / 2.0),
            (rect.top() + 2.0)..=(rect.bottom() - 4.0),
        );
        let response = ui
            .interact(cell, Id::unique(("task_header", i)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(key.tooltip());
        if response.clicked() {
            if *sort == key {
                *descending = !*descending;
            } else {
                *sort = key;
                *descending = key.starts_descending();
            }
        }
        let active = *sort == key;
        let hover = widgets::fade(ui, response.id, response.hovered());
        let on = widgets::fade(ui, response.id.with("on"), active);
        let mut fill = Color32::TRANSPARENT;
        if on > 0.0 {
            fill = widgets::blend(fill, pal.accent.gamma_multiply(0.16), on);
        }
        if hover > 0.0 {
            fill = widgets::blend(fill, pal.text.gamma_multiply(0.07), hover * (1.0 - on * 0.5));
        }
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(cell, CornerRadius::same(4), fill);
        }
        let color = if active { pal.text } else { widgets::blend(pal.muted, pal.text, hover) };
        let galley = fitted(ui, key.title(), font.clone(), color, x1 - x0 - 14.0);
        let width = galley.size().x;
        let y = cell.center().y;
        paint_cell(ui, (x0, x1), y, right, galley);
        if active {
            let x = if right { x1 - width - 9.0 } else { x0 + width + 9.0 };
            sort_arrow(ui, Pos2::new(x, y), *descending, pal.accent);
        }
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, pal.border),
    );
}

/// Fonts for the rows, resolved once per frame instead of once per cell.
struct RowFonts {
    body: FontId,
    bold: FontId,
}

impl RowFonts {
    fn new(ui: &Ui) -> Self {
        let body = TextStyle::Body.resolve(ui.style());
        Self {
            bold: FontId::new(body.size, semibold()),
            body,
        }
    }
}

/// One process. The whole row is a single clickable widget, painted directly.
fn process_row(
    ui: &mut Ui,
    pal: &Palette,
    fonts: &RowFonts,
    p: &ProcInfo,
    selected: bool,
    ram: u64,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        Sense::hover(),
    );
    // A stable id per process keeps its fades while the list scrolls or re-sorts.
    let response = ui.interact(rect, Id::unique(("task_row", p.pid)), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = widgets::fade(ui, response.id, response.hovered());
    let on = widgets::fade(ui, response.id.with("on"), selected);
    let row = rect.shrink2(egui::vec2(0.0, 1.0));
    let mut fill = Color32::TRANSPARENT;
    if on > 0.0 {
        fill = widgets::blend(fill, pal.accent.gamma_multiply(0.22), on);
    }
    if hover > 0.0 {
        fill = widgets::blend(fill, pal.text.gamma_multiply(0.06), hover);
    }
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(row, CornerRadius::same(4), fill);
    }
    if on > 0.0 {
        let bar = Rect::from_min_size(
            Pos2::new(row.left() + 2.0, row.center().y - 7.0 * on),
            egui::vec2(3.0, 14.0 * on),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), pal.accent);
    }

    let spans = column_spans(rect);
    let y = rect.center().y;
    let share = p.memory as f64 / ram as f64;
    let heavy_cpu = p.cpu >= HEAVY_CPU;
    let width = |i: usize| spans[i].1 - spans[i].0;

    let name_font = if heavy_cpu || share >= MEMORY_WARN {
        fonts.bold.clone()
    } else {
        fonts.body.clone()
    };
    paint_cell(ui, spans[0], y, false, fitted(ui, &p.name, name_font, pal.text, width(0)));
    paint_cell(
        ui,
        spans[1],
        y,
        true,
        fitted(ui, &p.pid.to_string(), fonts.body.clone(), pal.muted, width(1)),
    );
    let kind = match p.kind {
        ProcKind::App => tr("App"),
        ProcKind::Background => tr("Background"),
    };
    paint_cell(ui, spans[2], y, false, fitted(ui, kind, fonts.body.clone(), pal.text, width(2)));
    let cpu_color = if heavy_cpu { pal.warn } else { pal.text };
    paint_cell(
        ui,
        spans[3],
        y,
        true,
        fitted(ui, &format!("{:.1}%", p.cpu), fonts.body.clone(), cpu_color, width(3)),
    );
    let (mem_font, mem_color) = if share >= MEMORY_HEAVY {
        (fonts.bold.clone(), pal.error)
    } else if share >= MEMORY_WARN {
        (fonts.body.clone(), pal.warn)
    } else {
        (fonts.body.clone(), pal.text)
    };
    paint_cell(
        ui,
        spans[4],
        y,
        true,
        fitted(ui, &fmt_bytes(p.memory), mem_font, mem_color, width(4)),
    );
    let status = if p.suspended {
        Some((tr("Suspended"), pal.warn))
    } else if p.protected {
        Some((tr("Protected"), pal.muted))
    } else {
        None
    };
    if let Some((text, color)) = status {
        paint_cell(ui, spans[5], y, false, fitted(ui, text, fonts.body.clone(), color, width(5)));
    }

    // Tooltips for the columns that need explaining.
    let x = response.hover_pos().map(|p| p.x);
    let over = |i: usize| x.is_some_and(|x| x >= spans[i].0 && x <= spans[i].1);
    if over(4) {
        let percent = format!("{:.1}", share * 100.0);
        response.on_hover_text(trf("{0}% of your RAM", &[&percent]))
    } else if over(5) && p.suspended {
        response.on_hover_text(tr("Paused: it gets no CPU time until it is resumed"))
    } else if over(5) && p.protected {
        response.on_hover_text(tr("System process: can only be trimmed"))
    } else {
        response
    }
}
