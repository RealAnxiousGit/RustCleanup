//! Windows settings (the four reversible switches) and the System Tools picker.
//! On Linux only Low Power Mode applies, so that is the one switch shown.

use eframe::egui::{self, RichText, Ui};

use super::theme::{Palette, semibold};
use super::widgets;
use crate::events::{AppEvent, Job, Jobs, TweakState};
#[cfg(windows)]
use crate::i18n::trf;
use crate::i18n::tr;
#[cfg(windows)]
use crate::sys::format::friendly_date;
use crate::sys::power;
use crate::sys::tools::TOOLS;
#[cfg(windows)]
use crate::sys::{copilot, telemetry, updates};

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
    #[cfg_attr(not(windows), allow(dead_code))]
    restart_explorer: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
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
    pub fn detect(jobs: &Jobs) {
        jobs.start(Job::DetectState, |log| {
            #[cfg(windows)]
            let state = TweakState {
                copilot_disabled: copilot::copilot_disabled(),
                telemetry_disabled: telemetry::telemetry_disabled(),
                updates_blocked: updates::forced_updates_blocked(),
                paused_until: updates::paused_until(),
                low_power: power::low_power_active(),
            };
            #[cfg(not(windows))]
            let state = TweakState {
                low_power: power::low_power_active(),
                ..Default::default()
            };
            log.send(AppEvent::State(state));
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

    /// The Linux version: Low Power Mode only.
    #[cfg(not(windows))]
    pub fn settings_ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &Jobs) {
        widgets::heading(ui, tr("Power"));
        if !self.loaded {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                widgets::note(ui, pal, tr("Reading the current settings."));
            });
        }
        ui.add_space(8.0);
        self.low_power_row(ui, pal, jobs);
    }

    fn low_power_row(&mut self, ui: &mut Ui, pal: &Palette, jobs: &Jobs) {
        let detail = if cfg!(windows) {
            tr("Caps the CPU at 60% plugged in and 50% on battery, and turns off turbo boost. Off restores your previous power plan.")
        } else {
            tr("Switches to the power saver profile. As root it also caps the CPU at 60% and turns off turbo boost. Off restores your previous profile.")
        };
        if let Some(enable) = self.row(
            ui,
            pal,
            jobs,
            Row { job: Job::LowPower, title: tr("Low Power Mode"), detail, on: self.state.low_power },
        ) {
            self.state.low_power = enable;
            jobs.start(Job::LowPower, move |log| {
                if enable { power::enable_low_power(log) } else { power::disable_low_power(log) }
            });
        }
    }

    #[cfg(windows)]
    pub fn settings_ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &Jobs) {
        widgets::heading(ui, tr("Windows settings"));
        if !self.loaded {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(pal.muted));
                widgets::note(ui, pal, tr("Reading the current settings from Windows."));
            });
        } else {
            widgets::note(
                ui,
                pal,
                tr("Switch any of these off again to restore the Windows default."),
            );
        }
        ui.add_space(8.0);

        if let Some(disable) = self.row(
            ui,
            pal,
            jobs,
            Row {
                job: Job::Copilot,
                title: tr("Disable Copilot"),
                detail: tr(
                    "Turns on the TurnOffWindowsCopilot policy and hides the taskbar button.",
                ),
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
            tr("Restart Explorer so it applies right away"),
        )
        .on_hover_text(tr("The taskbar and open folder windows disappear for a moment"));
        widgets::divider(ui);

        if let Some(disable) = self.row(
            ui,
            pal,
            jobs,
            Row { job: Job::Telemetry, title: tr("Disable Telemetry"), detail: tr("Sets AllowTelemetry to 0, stops DiagTrack and dmwappushservice, and turns off the data collection tasks."), on: self.state.telemetry_disabled },
        ) {
            self.state.telemetry_disabled = disable;
            jobs.start(Job::Telemetry, move |log| telemetry::set_telemetry_disabled(log, disable));
        }
        widgets::divider(ui);

        if let Some(block) = self.row(
            ui,
            pal,
            jobs,
            Row { job: Job::Updates, title: tr("No Forced Updates"), detail: tr("Windows Update asks before installing and never restarts on its own while you are signed in."), on: self.state.updates_blocked },
        ) {
            let pause = self.pause_updates;
            self.state.updates_blocked = block;
            jobs.start(Job::Updates, move |log| updates::set_forced_updates_blocked(log, block, pause));
        }
        let pause_label = trf("Also pause updates for {0} days", &[&updates::PAUSE_DAYS]);
        ui.add_enabled(
            !self.state.updates_blocked,
            egui::Checkbox::new(&mut self.pause_updates, pause_label),
        )
        .on_hover_text(tr("Windows will not download any updates until then"));
        if let Some(until) = &self.state.paused_until {
            ui.label(
                RichText::new(trf(
                    "Updates are paused until {0}.",
                    &[&friendly_date(until)],
                ))
                .color(pal.warn),
            );
        }
        widgets::divider(ui);

        self.low_power_row(ui, pal, jobs);
    }

    pub fn tools_ui(&mut self, ui: &mut Ui, pal: &Palette, jobs: &Jobs) {
        widgets::heading(ui, tr("System tools"));
        widgets::note(
            ui,
            pal,
            if cfg!(windows) {
                tr("Opens the built-in Windows tool you pick.")
            } else {
                tr("Opens the desktop app you pick.")
            },
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("tools")
                .width((ui.available_width() - 80.0).max(140.0))
                .selected_text(tr(TOOLS[self.tool_idx].label))
                .show_ui(ui, |ui| {
                    for (i, tool) in TOOLS.iter().enumerate() {
                        ui.selectable_value(&mut self.tool_idx, i, tr(tool.label))
                            .on_hover_text(tr(tool.description));
                    }
                });
            if widgets::button(ui, tr("Open"), true)
                .on_hover_text(tr("Open the selected tool"))
                .clicked()
            {
                let tool = &TOOLS[self.tool_idx];
                match tool.launch() {
                    Ok(()) => jobs.log.info(crate::i18n::trf("Opened {0}.", &[&tr(tool.label)])),
                    Err(e) => jobs.log.error(e),
                }
            }
        });
        widgets::note(ui, pal, tr(TOOLS[self.tool_idx].description));
    }
}
