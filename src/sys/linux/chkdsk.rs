//! Scan Disk on Linux: runs the right checker for the filesystem in the
//! background and streams its output line by line.
//!
//! Every filesystem in the list is mounted, so only checks that are safe on
//! a mounted filesystem are used:
//!
//! | Filesystem   | Read-only check            | Online scan            |
//! |--------------|----------------------------|------------------------|
//! | ext2/3/4     | `e2fsck -n -f`             |                        |
//! | Btrfs        | `btrfs scrub start -B -r`  | `btrfs scrub start -B` |
//! | XFS          | `xfs_scrub -n`             | `xfs_scrub`            |
//! | FAT, exFAT   | `fsck.fat -n`, `fsck.exfat -n` |                    |
//! | F2FS         | `fsck.f2fs --dry-run`      |                        |
//! | NTFS         | `ntfsfix -n`               |                        |
//!
//! `-n` and friends open the device read-only and answer "no" to every
//! repair, so nothing on the disk changes.

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use super::command::{hidden_command, which};
use super::disk::Drive;
use super::privilege::is_elevated;
use crate::i18n::tr;

/// Which pass to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    /// Checks without changing anything.
    ReadOnly,
    /// Btrfs and XFS: checks and repairs while the filesystem stays mounted.
    OnlineScan,
}

impl ScanMode {
    pub const ALL: [ScanMode; 2] = [ScanMode::ReadOnly, ScanMode::OnlineScan];

    pub fn label(self) -> &'static str {
        match self {
            ScanMode::ReadOnly => tr("Read-only check"),
            ScanMode::OnlineScan => tr("Online scan (Btrfs and XFS only)"),
        }
    }

    /// Short form for window titles.
    pub fn short(self) -> &'static str {
        match self {
            ScanMode::ReadOnly => "read-only",
            ScanMode::OnlineScan => "online scan",
        }
    }

    pub fn supports(self, drive: &Drive) -> bool {
        match self {
            ScanMode::ReadOnly => drive.scannable,
            ScanMode::OnlineScan => drive.scannable && drive.is_ntfs(),
        }
    }
}

/// Messages streamed from a running scan to the UI.
#[derive(Debug)]
pub enum ScanEvent {
    Line(String),
    /// A carriage-return terminated progress line that replaces the previous one.
    Progress(String),
    /// The checker exited. `Ok(code)` holds its exit code.
    Finished(Result<i32, String>),
}

/// Handle to a running checker, used for cancelling.
pub struct ScanHandle {
    pub pid: u32,
}

/// The program and arguments that check `drive` in `mode`.
fn checker(drive: &Drive, mode: ScanMode) -> Option<(&'static str, Vec<String>)> {
    let dev = drive.device.clone();
    let mnt = drive.root.clone();
    let online = mode == ScanMode::OnlineScan;
    let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    Some(match drive.file_system.as_str() {
        "ext2" | "ext3" | "ext4" if !online => {
            ("e2fsck", [s(&["-n", "-f", "-v", "-C", "0"]), vec![dev]].concat())
        }
        "btrfs" if online => ("btrfs", [s(&["scrub", "start", "-B", "-d"]), vec![mnt]].concat()),
        "btrfs" => ("btrfs", [s(&["scrub", "start", "-B", "-d", "-r"]), vec![mnt]].concat()),
        "xfs" if online => ("xfs_scrub", [s(&["-v"]), vec![mnt]].concat()),
        "xfs" => ("xfs_scrub", [s(&["-n", "-v"]), vec![mnt]].concat()),
        "vfat" | "msdos" if !online => ("fsck.fat", [s(&["-n", "-v"]), vec![dev]].concat()),
        "exfat" if !online => ("fsck.exfat", [s(&["-n", "-v"]), vec![dev]].concat()),
        "f2fs" if !online => ("fsck.f2fs", [s(&["--dry-run"]), vec![dev]].concat()),
        "ntfs" | "ntfs3" if !online => ("ntfsfix", [s(&["-n"]), vec![dev]].concat()),
        _ => return None,
    })
}

/// Starts the checker for `drive` and streams output through `tx`. `notify`
/// is called after each event so the UI can repaint immediately.
pub fn start_chkdsk(
    drive: &Drive,
    mode: ScanMode,
    tx: Sender<ScanEvent>,
    notify: impl Fn() + Send + Clone + 'static,
) -> Result<ScanHandle, String> {
    if !is_elevated() {
        return Err(tr("Checking a disk needs root. Restart RustCleanup as root first.").into());
    }
    let (program, args) = checker(drive, mode)
        .ok_or_else(|| format!("no checker for {} in this mode", drive.file_system))?;
    if which(program).is_none() {
        return Err(format!("{program} is not installed"));
    }
    let mut cmd = hidden_command(program);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child: Child = cmd
        .spawn()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("stderr unavailable")?;

    let _ = tx.send(ScanEvent::Line(format!("$ {program} {}", args.join(" "))));
    if !drive.is_ntfs() {
        let _ = tx.send(ScanEvent::Line(
            tr("The filesystem is mounted, so the check is read-only and may report changes that are still being written.")
                .into(),
        ));
    }

    let out_reader = spawn_reader(stdout, tx.clone(), notify.clone());
    let err_reader = spawn_reader(stderr, tx.clone(), notify.clone());

    thread::Builder::new()
        .name("scan-wait".into())
        .spawn(move || {
            let result = child.wait().map_err(|e| e.to_string()).map(|s| {
                use std::os::unix::process::ExitStatusExt;
                // Stopped by a signal (Stop check) reads as "cancelled".
                s.code().unwrap_or(if s.signal().is_some() { 32 } else { -1 })
            });
            let _ = out_reader.join();
            let _ = err_reader.join();
            let _ = tx.send(ScanEvent::Finished(result));
            notify();
        })
        .map_err(|e| format!("could not start the watcher thread: {e}"))?;

    Ok(ScanHandle { pid })
}

/// Reads a pipe and splits it into lines on `\n` and `\r`; a bare `\r`
/// redraws a progress line.
fn spawn_reader(
    mut pipe: impl Read + Send + 'static,
    tx: Sender<ScanEvent>,
    notify: impl Fn() + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim_end().to_string();
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            pending.extend_from_slice(&buf[..n]);
            let mut start = 0;
            let mut i = 0;
            while i < pending.len() {
                let c = pending[i];
                if c == b'\n' || c == b'\r' {
                    let carriage = c == b'\r';
                    if carriage && i + 1 >= pending.len() {
                        break; // maybe the first half of "\r\n"
                    }
                    let next_is_lf = pending.get(i + 1) == Some(&b'\n');
                    let line = text(&pending[start..i]);
                    let event = if carriage && !next_is_lf {
                        ScanEvent::Progress(line)
                    } else {
                        ScanEvent::Line(line)
                    };
                    if tx.send(event).is_err() {
                        return;
                    }
                    if carriage && next_is_lf {
                        i += 1;
                    }
                    start = i + 1;
                }
                i += 1;
            }
            pending.drain(..start);
            notify();
        }
        if !pending.is_empty() {
            for line in String::from_utf8_lossy(&pending).lines() {
                let _ = tx.send(ScanEvent::Line(line.trim_end_matches('\r').to_string()));
            }
            notify();
        }
    })
}

/// Human explanation of the fsck family's exit codes.
pub fn describe_exit(code: i32) -> &'static str {
    match code {
        0 => tr("No errors were found."),
        1 | 2 => tr("Errors were found and fixed."),
        3 | 4 => tr(
            "Errors were found. Repair them from a live USB while the drive is not mounted.",
        ),
        8 | 16 => tr("The check could not run on this drive."),
        32 => tr("Disk check cancelled."),
        _ => tr("The check ended with an unexpected exit code."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct Trickle(Vec<u8>, usize);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = (self.0.len() - self.1).min(3).min(buf.len());
            buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
            self.1 += n;
            Ok(n)
        }
    }

    #[test]
    fn splits_lines_and_progress() {
        let (tx, rx) = mpsc::channel();
        spawn_reader(
            Trickle(b"Pass 1\nProgress 10%\rProgress 55%\rDone.\r\ntail".to_vec(), 0),
            tx,
            || {},
        )
        .join()
        .unwrap();
        let events: Vec<String> = rx
            .try_iter()
            .map(|e| match e {
                ScanEvent::Line(l) => format!("L:{l}"),
                ScanEvent::Progress(p) => format!("P:{p}"),
                ScanEvent::Finished(_) => "F".into(),
            })
            .collect();
        assert_eq!(
            events,
            ["L:Pass 1", "P:Progress 10%", "P:Progress 55%", "L:Done.", "L:tail"]
        );
    }
}
