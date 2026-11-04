//! RAM stress test for development builds (debug builds, or release builds
//! with `--features stress-test`). Never part of a normal release.
//!
//! It starts helper copies of this exe ("hogs") that each fill a block of
//! RAM with incompressible data, then runs Clean RAM several times. Before
//! every run the hogs touch all their pages again, so each run has the same
//! amount to reclaim. Each run reports how much the hogs' working sets
//! shrank, how much available RAM grew, and how long every stage took.
//!
//! From a terminal:
//!
//! ```text
//! RustCleanup.exe --stress-ram [--size-mb 4096] [--hogs 4] [--runs 5] [--purge] [--csv out.csv]
//! ```
//!
//! `scripts/stress-ram.ps1` (Windows) and `scripts/stress-ram.sh` (Linux)
//! build the debug exe and run this for you. On Linux `--purge` drops the
//! page cache (root only), and the hogs' working sets come from
//! `/proc/<pid>/statm`.
//! The strings here are English only on purpose: this is a developer tool.

use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};

use super::format::fmt_bytes;
use super::memory;
use crate::events::{AppEvent, Logger};

/// Starts the stress test from the command line.
pub const STRESS_ARG: &str = "--stress-ram";
/// Internal: run as a hog that holds this many MB.
const HOG_ARG: &str = "--stress-hog";

const MB: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Config {
    /// Total RAM the hogs hold, split evenly between them.
    pub size_mb: u64,
    pub hogs: u32,
    pub runs: u32,
    pub purge_standby: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            size_mb: 2048,
            hogs: 4,
            runs: 5,
            purge_standby: false,
        }
    }
}

/// One Clean RAM pass under load.
#[derive(Clone, Copy, Debug)]
pub struct RunResult {
    pub run: u32,
    /// Time the hogs took to fault their pages back in before this run.
    pub refill: Duration,
    pub hogs_before: u64,
    pub hogs_after: u64,
    pub clean: memory::CleanReport,
    /// Wall time of the whole Clean RAM call (includes its 500 ms settle pause).
    pub total: Duration,
}

impl RunResult {
    pub fn reclaimed(&self) -> u64 {
        self.hogs_before.saturating_sub(self.hogs_after)
    }

    pub fn line(&self) -> String {
        format!(
            "Run {}: hogs {} -> {} ({} reclaimed), available +{}, trim {} ms, purge {} ms, total {} ms, refill {} ms",
            self.run,
            fmt_bytes(self.hogs_before),
            fmt_bytes(self.hogs_after),
            fmt_bytes(self.reclaimed()),
            fmt_bytes(self.clean.freed()),
            self.clean.trim_time.as_millis(),
            self.clean.purge_time.as_millis(),
            self.total.as_millis(),
            self.refill.as_millis(),
        )
    }

    fn csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{}",
            self.run,
            self.hogs_before,
            self.hogs_after,
            self.reclaimed(),
            self.clean.freed(),
            self.clean.trim_time.as_millis(),
            self.clean.purge_time.as_millis(),
            self.total.as_millis(),
            self.refill.as_millis(),
            self.clean.trimmed,
            self.clean.skipped,
        )
    }
}

const CSV_HEADER: &str = "run,hogs_before_bytes,hogs_after_bytes,reclaimed_bytes,available_gain_bytes,trim_ms,purge_ms,total_ms,refill_ms,processes_trimmed,processes_skipped";

/// Averages over all runs, for the last log line.
pub fn summary(results: &[RunResult]) -> String {
    if results.is_empty() {
        return "No runs finished.".into();
    }
    let n = results.len() as u64;
    let avg = |f: &dyn Fn(&RunResult) -> u64| results.iter().map(f).sum::<u64>() / n;
    let worst = results.iter().map(|r| r.total).max().unwrap_or_default();
    format!(
        "Average over {n} runs: {} reclaimed, available +{}, trim {} ms, total {} ms (slowest {} ms)",
        fmt_bytes(avg(&|r| r.reclaimed())),
        fmt_bytes(avg(&|r| r.clean.freed())),
        avg(&|r| r.clean.trim_time.as_millis() as u64),
        avg(&|r| r.total.as_millis() as u64),
        worst.as_millis(),
    )
}

struct Hog {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Hog {
    fn spawn(mb: u64) -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(exe);
        cmd.args([HOG_ARG, &mb.to_string()]);
        // The exe's manifest asks for Administrator rights; a hog doesn't
        // need them, and an unelevated parent couldn't start it otherwise.
        #[cfg(windows)]
        cmd.env("__COMPAT_LAYER", "RunAsInvoker");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start a hog: {e}"))?;
        let stdin = child.stdin.take().ok_or("hog has no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("hog has no stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    /// Waits for the hog's "ready" line.
    fn wait_ready(&mut self) -> Result<(), String> {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .map_err(|e| format!("hog stopped answering: {e}"))?;
        if line.trim() == "ready" {
            Ok(())
        } else {
            Err(format!("hog failed: {}", line.trim()))
        }
    }

    fn touch(&mut self) -> Result<(), String> {
        writeln!(self.stdin, "touch")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("hog stopped answering: {e}"))
    }

    #[cfg(target_os = "linux")]
    fn working_set(&self) -> u64 {
        // statm: size, resident, shared, ... in pages.
        let resident = std::fs::read_to_string(format!("/proc/{}/statm", self.child.id()))
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .unwrap_or(0);
        // SAFETY: plain sysconf query.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as u64;
        resident * page
    }

    #[cfg(windows)]
    fn working_set(&self) -> u64 {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let handle = HANDLE(self.child.as_raw_handle());
        // SAFETY: the Child owns a live process handle with full access.
        match unsafe { GetProcessMemoryInfo(handle, &mut counters, counters.cb) } {
            Ok(()) => counters.WorkingSetSize as u64,
            Err(_) => 0,
        }
    }
}

impl Drop for Hog {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Keeps the stress test from pushing the PC into heavy paging: the hogs
/// may take at most 80% of the RAM that is available right now.
fn safe_size_mb(requested: u64) -> u64 {
    let available = memory::snapshot()
        .physical
        .map_or(0, |p| p.available / MB);
    requested.min(available * 4 / 5).max(64)
}

/// Runs the whole test. `progress` gets each run as it finishes.
pub fn run(
    cfg: &Config,
    log: &Logger,
    mut progress: impl FnMut(&RunResult),
) -> Result<Vec<RunResult>, String> {
    let hogs = cfg.hogs.clamp(1, 64);
    let size_mb = safe_size_mb(cfg.size_mb);
    if size_mb < cfg.size_mb {
        log.warn(format!(
            "Stress test: only {size_mb} MB of the requested {} MB fits in available RAM, using {size_mb} MB.",
            cfg.size_mb
        ));
    }
    let per_hog = (size_mb / hogs as u64).max(16);
    log.info(format!(
        "Stress test: {hogs} hogs x {per_hog} MB, {} runs, standby purge {}.",
        cfg.runs,
        if cfg.purge_standby { "on" } else { "off" }
    ));

    let fill_started = Instant::now();
    let mut pool = (0..hogs)
        .map(|_| Hog::spawn(per_hog))
        .collect::<Result<Vec<_>, _>>()?;
    for hog in &mut pool {
        hog.wait_ready()?;
    }
    log.info(format!(
        "Stress test: hogs hold {} after {} ms.",
        fmt_bytes(pool.iter().map(Hog::working_set).sum()),
        fill_started.elapsed().as_millis()
    ));

    let mut results = Vec::new();
    for run in 1..=cfg.runs.max(1) {
        let refill_started = Instant::now();
        if run > 1 {
            for hog in &mut pool {
                hog.touch()?;
            }
            for hog in &mut pool {
                hog.wait_ready()?;
            }
        }
        let refill = if run > 1 {
            refill_started.elapsed()
        } else {
            Duration::ZERO
        };
        let hogs_before = pool.iter().map(Hog::working_set).sum();
        let started = Instant::now();
        let clean = memory::clean_ram_report(log, cfg.purge_standby)?;
        let total = started.elapsed();
        let result = RunResult {
            run,
            refill,
            hogs_before,
            hogs_after: pool.iter().map(Hog::working_set).sum(),
            clean,
            total,
        };
        progress(&result);
        results.push(result);
    }
    Ok(results)
}

/// The hog side: hold `mb` MB of incompressible data, refault it on "touch",
/// and exit when stdin closes.
fn hog_main(mb: u64) -> i32 {
    let bytes = (mb * MB) as usize;
    let mut block = Vec::new();
    if block.try_reserve_exact(bytes).is_err() {
        println!("could not allocate {mb} MB");
        return 1;
    }
    // A random 1 MB pattern, rotated per MB, so memory compression can't
    // shrink the pages and every page differs from its neighbours.
    let mut seed = 0x9E37_79B9_7F4A_7C15u64 ^ std::process::id() as u64;
    let pattern: Vec<u8> = (0..MB / 8)
        .flat_map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed.to_le_bytes()
        })
        .collect();
    while block.len() < bytes {
        let start = block.len();
        let take = (bytes - start).min(pattern.len());
        let shift = (start / pattern.len() * 4093) % pattern.len();
        let first = (pattern.len() - shift).min(take);
        block.extend_from_slice(&pattern[shift..shift + first]);
        block.extend_from_slice(&pattern[..take - first]);
    }
    println!("ready");
    let _ = std::io::stdout().flush();

    let mut sum = 0u64;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim() == "touch" {
            // Read one byte per page to fault the trimmed pages back in.
            for page in block.iter().step_by(4096) {
                sum = sum.wrapping_add(*page as u64);
            }
            std::hint::black_box(sum);
            println!("ready");
            let _ = std::io::stdout().flush();
        }
    }
    0
}

/// Handles `--stress-ram` and `--stress-hog`. Returns the exit code when
/// one of them ran, so `main` can stop there.
pub fn cli(args: &[String]) -> Option<i32> {
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    if let Some(mb) = value(HOG_ARG) {
        return Some(hog_main(mb.parse().unwrap_or(256)));
    }
    if !args.iter().any(|a| a == STRESS_ARG) {
        return None;
    }
    attach_console();

    let defaults = Config::default();
    let number = |name: &str, default: u64| {
        value(name)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(default)
    };
    let cfg = Config {
        size_mb: number("--size-mb", defaults.size_mb),
        hogs: number("--hogs", defaults.hogs as u64) as u32,
        runs: number("--runs", defaults.runs as u64) as u32,
        purge_standby: args.iter().any(|a| a == "--purge"),
    };
    let csv_path = value("--csv").cloned();

    // Clean RAM reports through a Logger; print what it sends.
    let (tx, rx) = mpsc::channel();
    let log = Logger::new(tx, eframe::egui::Context::default());
    let printer = std::thread::spawn(move || {
        for event in rx {
            if let AppEvent::Log(entry) = event {
                println!("[{}] {:?}: {}", entry.time, entry.level, entry.message);
            }
        }
    });

    let result = run(&cfg, &log, |r| println!("{}", r.line()));
    drop(log);
    let _ = printer.join();
    match result {
        Ok(results) => {
            println!("{}", summary(&results));
            if let Some(path) = csv_path {
                let mut text = String::from(CSV_HEADER);
                for r in &results {
                    text.push('\n');
                    text.push_str(&r.csv());
                }
                text.push('\n');
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("Could not write {path}: {e}");
                    return Some(1);
                }
                println!("Wrote {path}");
            }
            Some(0)
        }
        Err(e) => {
            eprintln!("Stress test failed: {e}");
            Some(1)
        }
    }
}

/// Release builds have no console of their own; borrow the terminal's.
fn attach_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // SAFETY: plain Win32 call; failure only means no output.
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}
