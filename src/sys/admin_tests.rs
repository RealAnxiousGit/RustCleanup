//! Tests for the features that need Administrator rights. They change real
//! system settings, so they are ignored by default and each one puts the
//! setting back the way it found it. Run them from an elevated prompt with:
//!
//! `cargo test -- --ignored --test-threads=1`

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::{chkdsk, copilot, disk, memory, power, privilege, startup, telemetry, updates};
use crate::events::{AppEvent, Level, Logger};

fn logger() -> (Logger, Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel();
    (Logger::new(tx, eframe::egui::Context::default()), rx)
}

/// Prints what the feature logged and returns the warnings and errors.
fn problems(rx: &Receiver<AppEvent>) -> Vec<String> {
    let mut bad = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::Log(e) = event {
            println!("  [{:?}] {}", e.level, e.message);
            if matches!(e.level, Level::Warn | Level::Error) {
                bad.push(e.message);
            }
        }
    }
    bad
}

fn require_admin() {
    assert!(privilege::is_elevated(), "run these tests as Administrator");
}

#[test]
#[ignore = "changes system settings; needs Administrator"]
fn telemetry_toggles_and_reverts() {
    require_admin();
    let (log, rx) = logger();
    let original = telemetry::telemetry_disabled();
    telemetry::set_telemetry_disabled(&log, !original).unwrap();
    assert_eq!(telemetry::telemetry_disabled(), !original);
    telemetry::set_telemetry_disabled(&log, original).unwrap();
    assert_eq!(telemetry::telemetry_disabled(), original);
    assert!(problems(&rx).is_empty());
}

#[test]
#[ignore = "changes system settings; needs Administrator"]
fn copilot_toggles_and_reverts() {
    require_admin();
    let (log, rx) = logger();
    let original = copilot::copilot_disabled();
    copilot::set_copilot_disabled(&log, !original, false).unwrap();
    assert_eq!(copilot::copilot_disabled(), !original);
    copilot::set_copilot_disabled(&log, original, false).unwrap();
    assert_eq!(copilot::copilot_disabled(), original);
    assert!(problems(&rx).is_empty());
}

#[test]
#[ignore = "changes system settings; needs Administrator"]
fn forced_updates_toggle_and_revert() {
    require_admin();
    let (log, rx) = logger();
    let original = updates::forced_updates_blocked();
    let paused = updates::paused_until().is_some();
    println!(
        "  before: blocked={original} paused_until={:?}",
        updates::paused_until()
    );
    updates::set_forced_updates_blocked(&log, !original, paused).unwrap();
    assert_eq!(updates::forced_updates_blocked(), !original);
    updates::set_forced_updates_blocked(&log, original, paused).unwrap();
    assert_eq!(updates::forced_updates_blocked(), original);
    println!(
        "  after: blocked={original} paused_until={:?}",
        updates::paused_until()
    );
    assert!(problems(&rx).is_empty());
}

#[test]
#[ignore = "changes system settings; needs Administrator"]
fn low_power_mode_restores_previous_plan() {
    require_admin();
    if power::low_power_active() {
        println!("  skipped: Low Power Mode is already on");
        return;
    }
    let (log, rx) = logger();
    let before = power::active_scheme().unwrap();
    power::enable_low_power(&log).unwrap();
    assert!(power::low_power_active());
    power::disable_low_power(&log).unwrap();
    assert!(!power::low_power_active());
    assert_eq!(power::active_scheme().unwrap(), before);
    assert!(problems(&rx).is_empty());
}

#[test]
#[ignore = "changes system settings; needs Administrator"]
fn startup_task_registers_and_removes() {
    require_admin();
    if startup::registered_path().is_some() {
        println!("  skipped: an Open on startup task already exists");
        return;
    }
    let (log, rx) = logger();
    startup::set_enabled(&log, true).unwrap();
    let exe = std::env::current_exe().unwrap();
    let registered = startup::registered_path().expect("task is registered");
    println!("  registered: {registered}");
    assert!(registered.contains(&*exe.to_string_lossy()));
    startup::set_enabled(&log, false).unwrap();
    assert!(startup::registered_path().is_none());
    assert!(problems(&rx).is_empty());
}

#[test]
#[ignore = "needs Administrator"]
fn clean_ram_purges_standby_list() {
    require_admin();
    let (log, rx) = logger();
    memory::clean_ram(&log, true).unwrap();
    assert!(problems(&rx).is_empty());
}

#[test]
#[ignore = "runs chkdsk for a few minutes; needs Administrator"]
fn read_only_chkdsk_on_system_drive() {
    require_admin();
    let drive = disk::list_drives()
        .into_iter()
        .find(|d| d.volume().eq_ignore_ascii_case("C:"))
        .unwrap();
    let (tx, rx) = mpsc::channel();
    chkdsk::start_chkdsk(&drive, chkdsk::ScanMode::ReadOnly, tx, || {}).unwrap();
    let started = Instant::now();
    let mut lines = 0;
    let code = loop {
        match rx
            .recv_timeout(Duration::from_secs(900))
            .expect("chkdsk finishes")
        {
            chkdsk::ScanEvent::Line(l) => {
                lines += 1;
                println!("  {l}");
            }
            chkdsk::ScanEvent::Progress(_) => {}
            chkdsk::ScanEvent::Finished(r) => break r.unwrap(),
        }
    };
    println!(
        "  exit {code} ({}) after {} s, {lines} lines",
        chkdsk::describe_exit(code),
        started.elapsed().as_secs()
    );
    assert!(lines > 0);
    assert!((0..=2).contains(&code), "chkdsk exit {code}");
}
