//! Runs `chkdsk` in the background and streams its output line by line.

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use super::command::{decode_console, hidden_command, looks_like_utf16};
use super::disk::Drive;
use crate::i18n::tr;

/// Which chkdsk pass to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    /// `chkdsk X:`: read-only check, never changes the disk.
    ReadOnly,
    /// `chkdsk X: /scan`: NTFS online scan; queues found problems for repair
    /// without taking the volume offline.
    OnlineScan,
}

impl ScanMode {
    pub const ALL: [ScanMode; 2] = [ScanMode::ReadOnly, ScanMode::OnlineScan];

    pub fn label(self) -> &'static str {
        match self {
            ScanMode::ReadOnly => tr("Read-only check"),
            ScanMode::OnlineScan => tr("Online scan (NTFS only)"),
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
    /// A completed line of output.
    Line(String),
    /// A carriage-return terminated progress line that replaces the previous one.
    Progress(String),
    /// chkdsk exited. `Ok(code)` holds its exit code.
    Finished(Result<i32, String>),
}

/// Handle to a running chkdsk process, used for cancelling.
pub struct ScanHandle {
    pub pid: u32,
}

/// Starts chkdsk on `drive` and streams output through `tx`. `notify` is
/// called after each event so the UI can repaint immediately.
pub fn start_chkdsk(
    drive: &Drive,
    mode: ScanMode,
    tx: Sender<ScanEvent>,
    notify: impl Fn() + Send + Clone + 'static,
) -> Result<ScanHandle, String> {
    let mut cmd = hidden_command("chkdsk.exe");
    cmd.arg(drive.volume());
    if mode == ScanMode::OnlineScan {
        cmd.arg("/scan");
    }
    // stdin is closed so chkdsk can never block waiting on a Y/N prompt.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child: Child = cmd
        .spawn()
        .map_err(|e| format!("could not start chkdsk: {e}"))?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("chkdsk stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("chkdsk stderr unavailable")?;

    let out_reader = spawn_reader(stdout, tx.clone(), notify.clone());
    let err_reader = spawn_reader(stderr, tx.clone(), notify.clone());

    thread::Builder::new()
        .name("chkdsk-wait".into())
        .spawn(move || {
            let result = child
                .wait()
                .map_err(|e| e.to_string())
                .map(|s| s.code().unwrap_or(-1));
            // Drain the pipes fully before reporting completion so no output is lost.
            let _ = out_reader.join();
            let _ = err_reader.join();
            let _ = tx.send(ScanEvent::Finished(result));
            notify();
        })
        .map_err(|e| format!("could not start chkdsk watcher thread: {e}"))?;

    Ok(ScanHandle { pid })
}

/// Reads a pipe in chunks and splits it into lines on `\n` and `\r`.
///
/// chkdsk redraws its progress line with a bare `\r`, so those become
/// `Progress` events that overwrite the last line instead of piling up.
fn spawn_reader(
    mut pipe: impl Read + Send + 'static,
    tx: Sender<ScanEvent>,
    notify: impl Fn() + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pending: Vec<u8> = Vec::new();
        let mut utf16: Option<bool> = None;
        let mut buf = [0u8; 4096];
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            pending.extend_from_slice(&buf[..n]);
            // Decide the encoding once, from a big enough sample to be reliable.
            if utf16.is_none() && pending.len() < 16 {
                continue;
            }
            let wide = *utf16.get_or_insert_with(|| looks_like_utf16(&pending));
            let unit = if wide { 2 } else { 1 };
            let is_at = |buf: &[u8], i: usize, b: u8| {
                buf.get(i) == Some(&b) && (!wide || buf.get(i + 1) == Some(&0))
            };

            // Emit every complete line currently buffered.
            let mut start = 0;
            let mut i = 0;
            while i + unit <= pending.len() {
                let is = |b: u8| is_at(&pending, i, b);
                if is(b'\n') || is(b'\r') {
                    let carriage = is(b'\r');
                    let text = decode_console(&pending[start..i]);
                    // "\r\n" is a normal line ending, not a progress redraw.
                    let next_is_lf = is_at(&pending, i + unit, b'\n');
                    let event = if carriage && !next_is_lf {
                        ScanEvent::Progress(text)
                    } else {
                        ScanEvent::Line(text)
                    };
                    // A "\r" right at the end of the buffer may be the first half
                    // of "\r\n": wait for more data before deciding.
                    if carriage && i + 2 * unit > pending.len() {
                        break;
                    }
                    if tx.send(event).is_err() {
                        return;
                    }
                    if carriage && next_is_lf {
                        i += unit; // skip the '\n' of "\r\n"
                    }
                    start = i + unit;
                }
                i += unit;
            }
            pending.drain(..start);
            notify();
        }
        // Whatever is left at EOF (including very short outputs that never
        // reached the detection threshold) is flushed as plain lines.
        if !pending.is_empty() {
            for line in decode_console(&pending).lines() {
                let _ = tx.send(ScanEvent::Line(line.trim_end_matches('\r').to_string()));
            }
            notify();
        }
    })
}

/// Human explanation of chkdsk's exit codes.
pub fn describe_exit(code: i32) -> &'static str {
    match code {
        0 => tr("No errors were found."),
        1 => tr("Errors were found and fixed."),
        2 => tr(
            "Disk cleanup was performed (e.g. garbage collection) or not done because /f was not given.",
        ),
        3 => tr(
            "Could not check the disk, errors could not be fixed, or errors were not fixed because /f was not given.",
        ),
        _ => tr("chkdsk ended with an unexpected exit code."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// A reader that hands out a few bytes at a time, to exercise lines
    /// (and "\r\n" pairs) split across read boundaries.
    struct Trickle(Vec<u8>, usize);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = (self.0.len() - self.1).min(3).min(buf.len());
            buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
            self.1 += n;
            Ok(n)
        }
    }

    fn collect(input: &[u8]) -> Vec<String> {
        let (tx, rx) = mpsc::channel();
        spawn_reader(Trickle(input.to_vec(), 0), tx, || {})
            .join()
            .unwrap();
        rx.try_iter()
            .map(|e| match e {
                ScanEvent::Line(l) => format!("L:{l}"),
                ScanEvent::Progress(p) => format!("P:{p}"),
                ScanEvent::Finished(_) => "F".into(),
            })
            .collect()
    }

    #[test]
    fn splits_lines_and_progress() {
        let events = collect(b"Stage 1\r\nProgress: 10%\rProgress: 55%\rDone.\r\ntail");
        assert_eq!(
            events,
            [
                "L:Stage 1",
                "P:Progress: 10%",
                "P:Progress: 55%",
                "L:Done.",
                "L:tail"
            ]
        );
    }

    #[test]
    fn decodes_utf16_stream() {
        let bytes: Vec<u8> = "Hi\r\nthere\r\n"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(collect(&bytes), ["L:Hi", "L:there"]);
    }
}
