//! Windows settings (the four reversible switches) and the System Tools picker.

use eframe::egui::{self, RichText, Ui};

use super::theme::{Palette, semibold};
use super::widgets;
use crate::events::{AppEvent, Job, Jobs, TweakState};
use crate::sys::format::friendly_date;
use crate::sys::tools::TOOLS;
use crate::sys::{copilot, power, telemetry, updates};

/// What one switch row shows.
struct Row {
    job: Job,
    title: &'static str,
    detail: &'static str,
    on: bool,
}

pub struct TweaksPanel {
    pub state: TweakState,
    /// False until the first read of the real system state arrives; the
    /// switches stay disabled until then so they never show a guess.
    pub loaded: bool,
    restart_explorer: bool,
    pause_updates: bool,
    tool_idx: usize,
}

impl TweaksPanel {
    pub fn new() -> Self {
        Self {
            state: TweakState::default(),
            loaded: false,
            restart_explorer: true,
            pause_updates: true,
            tool_idx: 0,
        }
    }

    /// Re-reads every switch's real state from the system, in the background.
    pub fn detect(jobs: &mut Jobs) {
        jobs.start(Job::DetectState, |log| {
            log.send(AppEvent::State(TweakState {
                copilot_disabled: copilot::copilot_disabled(),
                telemetry_disabled: telemetry::telemetry_disabled(),
                updates_blocked: updates::forced_updates_blocked(),
                paused_until: updates::paused_until(),
                low_power: power::low_power_active(),
            }));
            Ok(())
        });
    }

    /// One switch row: title and explanation on the left, switch on the right.
    /// Returns the requested new value when clicked.
    fn row(&self, ui: &mut Ui, pal: &Palette, jobs: &Jobs, row: Row) -> Option<bool> {
        let Row {
            job,
            title,
            detail,
            on,
        } = row;
        let busy = jobs.is_busy(job);
        let mut result = None;
        ui.horizontal(|ui| {
            let text_width = (ui.available_width() - 64.0).max(120.0);
            ui.vertical(|ui| {
                ui.set_width(text_width);
                ui.label(RichText::new(title).family(semibold()));
                widgets::note(ui, pal, detail);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::switch(ui, pal, on, !busy && self.loaded, title).clicked() {
                    result = Some(!on);
                }
                if busy {
                    ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                }
            });
        });
        result
    }

    pub fn settings_ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &mut Jobs) {
        widgets::heading(ui, "Windows settings");
        if !self.loaded {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                widgets::note(ui, pal, "Reading the current settings from Windows.");
            });
        } else {
            widgets::note(
                ui,
                pal,
                "Switch any of these off again to restore the Windows default.",
            );
        }
        ui.add_space(8.0);

        if let Some(disable) = self.row(
            ui,
            pal,
            jobs,
            Row {
                job: Job::Copilot,
                title: "Disable Copilot",
                detail: "Turns on the TurnOffWindowsCopilot policy and hides the taskbar button.",
                on: self.state.copilot_disabled,
            },
        ) {
            let restart = self.restart_explorer;
            self.state.copilot_disabled = disable;
            jobs.start(Job::Copilot, move |log| {
                copilot::set_copilot_disabled(log, disable, restart)
            });
        }
        ui.checkbox(
            &mut self.restart_explorer,
            "Restart Explorer so it applies right away",
        );
        ui.separator();

        if let Some(disable) = self.row(
            ui,
            pal,
            jobs,
            Row { job: Job::Telemetry, title: "Disable Telemetry", detail: "Sets AllowTelemetry to 0, stops DiagTrack and dmwappushservice, and turns off the data collection tasks.", on: self.state.telemetry_disabled },
        ) {
            self.state.telemetry_disabled = disable;
            jobs.start(Job::Telemetry, move |log| telemetry::set_telemetry_disabled(log, disable));
        }
        ui.separator();

        if let Some(block) = self.row(
            ui,
            pal,
            jobs,
            Row { job: Job::Updates, title: "No Forced Updates", detail: "Windows Update asks before installing and never restarts on its own while you are signed in.", on: self.state.updates_blocked },
        ) {
            let pause = self.pause_updates;
            self.state.updates_blocked = block;
            jobs.start(Job::Updates, move |log| updates::set_forced_updates_blocked(log, block, pause));
        }
        let pause_label = format!("Also pause updates for {} days", updates::PAUSE_DAYS);
        ui.add_enabled(
            !self.state.updates_blocked,
            egui::Checkbox::new(&mut self.pause_updates, pause_label),
        );
        if let Some(until) = &self.state.paused_until {
            ui.label(
                RichText::new(format!(
                    "Updates are paused until {}.",
                    friendly_date(until)
                ))
                .color(pal.warn),
            );
        }
        ui.separator();

        if let Some(enable) = self.row(
            ui,
            pal,
            jobs,
            Row { job: Job::LowPower, title: "Low Power Mode", detail: "Caps the CPU at 60% plugged in and 50% on battery, and turns off turbo boost. Off restores your previous power plan.", on: self.state.low_power },
        ) {
            self.state.low_power = enable;
            jobs.start(Job::LowPower, move |log| {
                if enable { power::enable_low_power(log) } else { power::disable_low_power(log) }
            });
        }
    }

    pub fn tools_ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &mut Jobs) {
        widgets::heading(ui, "System tools");
        widgets::note(ui, pal, "Opens the built-in Windows tool you pick.");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("tools")
                .width((ui.available_width() - 80.0).max(140.0))
                .selected_text(TOOLS[self.tool_idx].label)
                .show_ui(ui, |ui| {
                    for (i, tool) in TOOLS.iter().enumerate() {
                        ui.selectable_value(&mut self.tool_idx, i, tool.label)
                            .on_hover_text(tool.description);
                    }
                });
            if widgets::button(ui, "Open", true).clicked() {
                let tool = &TOOLS[self.tool_idx];
                match tool.launch() {
                    Ok(()) => jobs.log.info(format!("Opened {}.", tool.label)),
                    Err(e) => jobs.log.error(e),
                }
            }
        });
        widgets::note(ui, pal, TOOLS[self.tool_idx].description);
    }
}
