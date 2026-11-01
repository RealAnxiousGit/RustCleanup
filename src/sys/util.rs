//! Small helpers shared by every system module: hidden child processes,
//! console-output decoding, token privileges and elevation checks.

use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Globalization::{MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TokenElevation,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PCWSTR};

/// `CREATE_NO_WINDOW`: run console tools without flashing a console window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `CP_OEMCP`: the console code page used by tools such as chkdsk, sc and powercfg.
const CP_OEMCP: u32 = 1;

/// Absolute path to a tool in `%SystemRoot%\System32`.
///
/// We always call system tools by full path so a stray `sc.exe` or
/// `powercfg.exe` earlier on `PATH` can never run with our admin token.
pub fn system32(exe: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(exe)
}

/// A `Command` for a System32 tool that never shows a console window.
pub fn hidden_command(exe: &str) -> Command {
    let mut cmd = Command::new(system32(exe));
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Output of a finished console tool, already decoded to UTF-8.
pub struct CmdOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CmdOutput {
    /// The most useful single line of output, for log messages.
    pub fn summary(&self) -> String {
        let text = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
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
        "WindowsPowerShell\\v1.0\\powershell.exe",
        &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
    )
}

/// Decodes console output. Most tools write in the OEM code page when
/// redirected, but a few write UTF-16LE, so we detect that first.
pub fn decode_console(bytes: &[u8]) -> String {
    if looks_like_utf16(bytes) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>().0.iter()
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
pub fn decode_oem(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
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

/// Closes a handle when dropped so early returns never leak it.
pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: we own this handle and close it exactly once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Returns true if the current process token is elevated (Administrator).
pub fn is_elevated() -> bool {
    // SAFETY: plain Win32 calls with correctly sized out-parameters.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok()
            && elevation.TokenIsElevated != 0
    }
}

/// Enables a privilege such as `SeDebugPrivilege` on our own token.
///
/// Administrators hold these privileges but they start disabled; several
/// memory and process APIs fail with "access denied" until enabled.
pub fn enable_privilege(name: &str) -> Result<(), String> {
    // SAFETY: plain Win32 calls; the token handle is closed by OwnedHandle.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token)
            .map_err(|e| format!("OpenProcessToken: {e}"))?;
        let token = OwnedHandle(token);

        let mut luid = LUID::default();
        LookupPrivilegeValueW(PCWSTR::null(), &HSTRING::from(name), &mut luid)
            .map_err(|e| format!("LookupPrivilegeValue({name}): {e}"))?;

        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
        };
        AdjustTokenPrivileges(token.0, false, Some(&privileges), 0, None, None)
            .map_err(|e| format!("AdjustTokenPrivileges({name}): {e}"))?;
        // AdjustTokenPrivileges "succeeds" even when the account lacks the privilege.
        if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
            return Err(format!("{name} is not held by this account (run as Administrator)"));
        }
        Ok(())
    }
}

/// Formats a byte count as a short human-readable string.
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// Current local time as `HH:MM:SS`, for log timestamps.
pub fn local_time_hms() -> String {
    // SAFETY: GetLocalTime has no failure mode.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

/// UTC timestamp `days` from now in the `YYYY-MM-DDTHH:MM:SSZ` form that
/// the Windows Update pause settings use.
pub fn utc_iso_in_days(days: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now + days * 86_400;
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem / 60) % 60, rem % 60)
}

/// Days since 1970-01-01 to a (year, month, day) civil date
/// (Howard Hinnant's algorithm, valid for the whole proleptic Gregorian range).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_758), (2026, 11, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn utf16_detection() {
        let utf16: Vec<u8> = "Hello".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert!(looks_like_utf16(&utf16));
        assert!(!looks_like_utf16(b"Hello world"));
        assert_eq!(decode_console(&utf16), "Hello");
    }

    #[test]
    fn bytes_format() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
