//! Work that runs on its own schedule rather than on a click: the update
//! check and startup-task check at launch, and the Auto-Clean RAM watcher.

use std::time::{Duration, Instant};

use crate::events::{AppEvent, Job, Jobs, Logger};
use crate::i18n::{tr, trf};
use crate::settings;
use crate::sys::{memory, startup, updater};

/// How often Auto-Clean RAM looks at memory use.
const AUTO_CLEAN_POLL: Duration = Duration::from_secs(5);
/// Auto-Clean RAM never runs more often than this, so a machine that stays
/// above the limit isn't trimmed over and over.
pub const AUTO_CLEAN_COOLDOWN: Duration = Duration::from_secs(120);

/// Checks GitHub for a newer release. With automatic updates on, installs
/// it and asks the app to restart; otherwise tells the app so it can ask.
pub fn check_updates(log: &Logger, manual: bool) -> Result<(), String> {
    if manual {
        log.info(tr("Checking for updates."));
    }
    let release = match updater::check() {
        Ok(Some(r)) => r,
        Ok(None) => {
            log.send(AppEvent::UpToDate { manual });
            return Ok(());
        }
        // A failed background check isn't worth an error on every launch.
        Err(e) if !manual => {
            log.info(trf("Could not check for updates: {0}", &[&e]));
            return Ok(());
        }
        Err(e) => return Err(trf("Could not check for updates: {0}", &[&e])),
    };
    if settings::get().auto_update && !manual {
        log.info(trf(
            "RustCleanup {0} is available and will be installed now.",
            &[&release.version],
        ));
        let exe = updater::install(&release, log)?;
        log.send(AppEvent::Restart(exe));
    } else {
        log.send(AppEvent::UpdateAvailable(release));
    }
    Ok(())
}

/// Starts the launch-time checks.
pub fn on_launch(jobs: &Jobs) {
    updater::remove_leftovers();
    let s = settings::get();
    if s.auto_update || s.update_notify {
        jobs.start(Job::UpdateCheck, |log| {
            // Let the window appear first; the check is never urgent.
            std::thread::sleep(Duration::from_secs(3));
            check_updates(log, false)
        });
    }
    jobs.start(Job::Startup, move |log| {
        startup::sync(log, s.open_on_startup)
    });
    start_auto_clean(jobs.clone());
}

/// Watches physical memory and runs Clean RAM when it passes the limit set
/// in Settings. Costs one memory query every few seconds.
fn start_auto_clean(jobs: Jobs) {
    let _ = std::thread::Builder::new()
        .name("auto-clean".into())
        .spawn(move || {
            let mut last: Option<Instant> = None;
            loop {
                std::thread::sleep(AUTO_CLEAN_POLL);
                let s = settings::get();
                if !s.auto_clean || last.is_some_and(|t| t.elapsed() < AUTO_CLEAN_COOLDOWN) {
                    continue;
                }
                let Some(physical) = memory::snapshot().physical else {
                    continue;
                };
                let percent = (physical.fraction() * 100.0).round() as u8;
                if percent < s.auto_clean_percent || jobs.is_busy(Job::CleanRam) {
                    continue;
                }
                last = Some(Instant::now());
                jobs.log.info(trf(
                    "Memory use reached {0}% (limit {1}%), so RAM is being cleaned automatically.",
                    &[&percent, &s.auto_clean_percent],
                ));
                let purge = s.purge_standby;
                jobs.start(Job::CleanRam, move |log| memory::clean_ram(log, purge));
            }
        });
}
