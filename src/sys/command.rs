//! Running Windows console tools hidden, and decoding what they print.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::Win32::Globalization::{MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};

/// `CREATE_NO_WINDOW`: run console tools without flashing a console window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `CP_OEMCP`: the console code page used by tools such as chkdsk, sc and powercfg.
const CP_OEMCP: u32 = 1;

/// Absolute path to a tool in `%SystemRoot%\System32`.
///
/// System tools are always called by full path so a stray `sc.exe` or
/// `powercfg.exe` earlier on `PATH` can never run with our admin token.
pub fn system32(exe: &str) -> PathBuf {
    windows_dir().join("System32").join(exe)
}

/// `%SystemRoot%`, normally `C:\Windows`.
pub fn windows_dir() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

/// A `Command` for a System32 tool that never shows a console window.
pub fn hidden_command(exe: &str) -> Command {
    let mut cmd = Command::new(system32(exe));
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Output of a finished console tool, already decoded.
pub struct CmdOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CmdOutput {
    /// The most useful single line of output, for log messages.
    pub fn summary(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        match (line.is_empty(), self.code) {
            (true, Some(c)) => format!("exit code {c}"),
            (true, None) => "terminated".to_string(),
            (false, _) => line.to_string(),
        }
    }
}

/// Runs a System32 tool hidden and waits for it to finish.
///
/// `Err` means the process could not be started at all; a non-zero exit is
/// reported through `CmdOutput::success` so callers can decide how bad it is.
pub fn run_hidden(exe: &str, args: &[&str]) -> Result<CmdOutput, String> {
    let out = hidden_command(exe)
        .args(args)
        .output()
        .map_err(|e| format!("could not start {exe}: {e}"))?;
    Ok(CmdOutput {
        success: out.status.success(),
        code: out.status.code(),
        stdout: decode_console(&out.stdout),
        stderr: decode_console(&out.stderr),
    })
}

/// Runs a PowerShell script hidden, without profiles or prompts.
pub fn run_powershell(script: &str) -> Result<CmdOutput, String> {
    run_hidden(
        r"WindowsPowerShell\v1.0\powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ],
    )
}

/// Decodes console output. Most tools write in the OEM code page when
/// redirected, but a few write UTF-16LE, so that is detected first.
pub fn decode_console(bytes: &[u8]) -> String {
    if looks_like_utf16(bytes) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .filter(|&u| u != 0xFEFF)
            .collect();
        return String::from_utf16_lossy(&units);
    }
    decode_oem(bytes)
}

/// True when every other byte is mostly zero, i.e. UTF-16LE ASCII text.
pub fn looks_like_utf16(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return bytes.starts_with(&[0xFF, 0xFE]);
    }
    let odd = bytes.len() / 2;
    let zeros = bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    bytes.starts_with(&[0xFF, 0xFE]) || zeros * 10 >= odd * 4
}

/// Converts OEM code-page bytes to a Rust string via `MultiByteToWideChar`.
fn decode_oem(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    // SAFETY: both buffers are valid slices; the first call only measures.
    unsafe {
        let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
        let len = MultiByteToWideChar(CP_OEMCP, flags, bytes, None);
        if len <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; len as usize];
        let written = MultiByteToWideChar(CP_OEMCP, flags, bytes, Some(&mut wide));
        wide.truncate(written.max(0) as usize);
        String::from_utf16_lossy(&wide)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_detection() {
        let utf16: Vec<u8> = "Hello"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert!(looks_like_utf16(&utf16));
        assert!(!looks_like_utf16(b"Hello world"));
        assert_eq!(decode_console(&utf16), "Hello");
    }
}
