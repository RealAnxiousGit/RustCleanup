//! Update checks against the GitHub releases of RealAnxiousGit/RustCleanup,
//! and replacing the running exe with a newer release.
//!
//! On Windows requests go through WinHTTP, so Windows' own TLS stack and
//! proxy settings are used; on Linux they go through curl. A download is only
//! installed if its SHA-256 matches the `.sha256` file published with it.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
#[cfg(windows)]
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
#[cfg(windows)]
use windows::core::HSTRING;

use crate::events::Logger;
use crate::i18n::{tr, trf};

const LATEST_RELEASE: &str =
    "https://api.github.com/repos/RealAnxiousGit/RustCleanup/releases/latest";
const ALL_RELEASES: &str =
    "https://api.github.com/repos/RealAnxiousGit/RustCleanup/releases?per_page=100";
/// The file this platform downloads from a release.
#[cfg(windows)]
fn asset() -> &'static str {
    "RustCleanup.exe"
}

/// The file this platform downloads from a release: the AppImage when
/// running from one, else the plain Linux binary.
#[cfg(target_os = "linux")]
fn asset() -> &'static str {
    if std::env::var_os("APPIMAGE").is_some_and(|p| !p.is_empty()) {
        "RustCleanup-x86_64.AppImage"
    } else {
        "RustCleanup-linux-x86_64"
    }
}
/// Test hook: pretend to be this version, to exercise the update flow
/// against the real releases page.
const VERSION_OVERRIDE: &str = "RUSTCLEANUP_PRETEND_VERSION";

/// A published release that is newer than the running copy.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// Release notes (Markdown) from GitHub.
    pub notes: String,
    pub page: String,
    exe_url: String,
    sha256_url: Option<String>,
}

/// The version this exe reports, as `major.minor.patch`.
pub fn current_version() -> String {
    std::env::var(VERSION_OVERRIDE).unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

/// `v2.0.10` -> `[2, 0, 10]`. Anything that isn't a number counts as 0.
fn parse_version(text: &str) -> Vec<u64> {
    text.trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(candidate), parse_version(current));
    a.resize(3, 0);
    b.resize(3, 0);
    a > b
}

/// Asks GitHub for the latest release. `Ok(None)` means this copy is up to date.
pub fn check() -> Result<Option<Release>, String> {
    let body = http_get(LATEST_RELEASE, |_, _| {})?;
    let json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| trf("GitHub sent a reply RustCleanup could not read: {0}", &[&e]))?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    if tag.is_empty() || !is_newer(tag, &current_version()) {
        return Ok(None);
    }
    let asset_url = |name: &str| {
        json["assets"].as_array().and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"].as_str() == Some(name))
                .and_then(|a| a["browser_download_url"].as_str())
                .map(str::to_string)
        })
    };
    let exe_url = asset_url(asset())
        .ok_or_else(|| trf("Release {0} has no {1} to download.", &[&tag, &asset()]))?;
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: json["body"]
            .as_str()
            .unwrap_or_default()
            .replace("\r\n", "\n"),
        page: json["html_url"].as_str().unwrap_or_default().to_string(),
        exe_url,
        sha256_url: asset_url(&format!("{}.sha256", asset())),
    }))
}

/// One version's release, for the Changelog window.
#[derive(Clone)]
pub struct ReleaseNotes {
    /// The version without its `v`, like `3.0.2`.
    pub version: String,
    /// The release's title, like `RustCleanup 3.0.2`.
    pub title: String,
    pub markdown: String,
    pub page: String,
    /// When it was published, like `November 2nd, 2026`.
    pub published: Option<String>,
}

impl ReleaseNotes {
    fn from_json(json: &serde_json::Value) -> Self {
        let version = json["tag_name"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches(['v', 'V'])
            .to_string();
        let title = match json["name"].as_str().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => format!("RustCleanup {version}"),
        };
        Self {
            version,
            title,
            markdown: json["body"]
                .as_str()
                .unwrap_or_default()
                .replace("\r\n", "\n"),
            page: json["html_url"].as_str().unwrap_or_default().to_string(),
            published: json["published_at"]
                .as_str()
                .and_then(crate::sys::format::long_date),
        }
    }
}

/// Every published release, newest first, for the Changelog window.
pub fn all_notes() -> Result<Vec<ReleaseNotes>, String> {
    let body = http_get(ALL_RELEASES, |_, _| {})?;
    let json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| trf("GitHub sent a reply RustCleanup could not read: {0}", &[&e]))?;
    let releases = json
        .as_array()
        .ok_or_else(|| trf("GitHub sent a reply RustCleanup could not read: {0}", &[&"not a list"]))?;
    let mut notes: Vec<(String, ReleaseNotes)> = releases
        .iter()
        .filter(|r| r["draft"].as_bool() != Some(true))
        .map(|r| {
            let at = r["published_at"].as_str().unwrap_or_default().to_string();
            (at, ReleaseNotes::from_json(r))
        })
        .collect();
    // GitHub lists by creation time; ISO 8601 dates sort as text.
    notes.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(notes.into_iter().map(|(_, n)| n).collect())
}

/// Downloads `release`, checks it, and swaps it in for the running exe.
/// Returns the path to start afterwards. The running copy keeps working
/// until it exits, and its old file is deleted on the next launch.
pub fn install(release: &Release, log: &Logger) -> Result<PathBuf, String> {
    let exe = own_file().map_err(|e| e.to_string())?;
    log.info(trf("Downloading RustCleanup {0}.", &[&release.version]));

    let mut last_tenth = 0;
    let bytes = http_get(&release.exe_url, |done, total| {
        if let Some(total) = total.filter(|t| *t > 0) {
            let tenth = done * 10 / total;
            if tenth > last_tenth && tenth < 10 {
                last_tenth = tenth;
                log.info(trf("Downloaded {0}%.", &[&(tenth * 10)]));
            }
        }
    })?;
    if bytes.len() < 1024 * 1024 || !bytes.starts_with(EXE_MAGIC) {
        return Err(if cfg!(windows) {
            tr("The download is not a valid RustCleanup.exe, so nothing was changed.")
        } else {
            tr("The download is not a valid RustCleanup for Linux, so nothing was changed.")
        }
        .into());
    }

    match &release.sha256_url {
        Some(url) => {
            let expected = String::from_utf8_lossy(&http_get(url, |_, _| {})?)
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if expected != actual {
                return Err(tr(
                    "The download's checksum does not match the release, so nothing was changed.",
                )
                .into());
            }
            log.info(tr("Checksum verified."));
        }
        None => log.warn(tr(
            "This release has no checksum file, so the download could not be verified.",
        )),
    }

    replace_exe(&exe, &bytes)?;
    log.success(trf(
        "RustCleanup {0} is installed. Restarting.",
        &[&release.version],
    ));
    Ok(exe)
}

/// How a program file starts: `MZ` on Windows, ELF on Linux.
#[cfg(windows)]
const EXE_MAGIC: &[u8] = b"MZ";
#[cfg(target_os = "linux")]
const EXE_MAGIC: &[u8] = b"\x7fELF";

/// The file an update replaces: the exe, or on Linux the AppImage when
/// running from one.
fn own_file() -> std::io::Result<PathBuf> {
    #[cfg(windows)]
    return std::env::current_exe();
    #[cfg(target_os = "linux")]
    return super::shell::app_exe();
}

/// `RustCleanup.exe` -> `RustCleanup.exe.old`.
fn with_suffix(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn old_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".old")
}

/// Windows can't overwrite a running exe, but it can rename it. The new
/// file is written next to it first, so a failure leaves the old one in place.
fn replace_exe(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let new = with_suffix(exe, ".new");
    let old = old_path(exe);
    let fail = |e: std::io::Error| trf("Could not install the update: {0}", &[&e]);
    std::fs::write(&new, bytes).map_err(fail)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(exe).map(|m| m.permissions().mode()).unwrap_or(0o755);
        if let Err(e) = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(mode | 0o111)) {
            let _ = std::fs::remove_file(&new);
            return Err(fail(e));
        }
        super::user::give_to_user(&new);
    }
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(fail(e));
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        // Put the original back so there is always a working RustCleanup.exe.
        let _ = std::fs::rename(&old, exe);
        return Err(fail(e));
    }
    Ok(())
}

/// True while the previous version from an update is still next to the exe,
/// which means this is the first launch since the update.
pub fn leftover_exists() -> bool {
    own_file().is_ok_and(|exe| old_path(&exe).exists())
}

/// Deletes the previous version left behind by an update.
///
/// Right after an update the old copy may still be exiting, and Windows keeps
/// its file locked until it is gone, so this keeps trying for a few seconds.
pub fn remove_leftovers() {
    let Ok(exe) = own_file() else {
        return;
    };
    let leftovers = [old_path(&exe), with_suffix(&exe, ".new")];
    if !leftovers.iter().any(|p| p.exists()) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("remove-leftovers".into())
        .spawn(move || {
            for _ in 0..30 {
                for path in &leftovers {
                    let _ = std::fs::remove_file(path);
                }
                if !leftovers.iter().any(|p| p.exists()) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
}

/// Closes a WinHTTP handle when dropped.
#[cfg(windows)]
struct Handle(*mut core::ffi::c_void);

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own this handle and close it once.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn open(handle: *mut core::ffi::c_void, what: &str) -> Result<Handle, String> {
    if handle.is_null() {
        Err(trf(
            "Could not reach GitHub ({0}): {1}",
            &[&what, &windows::core::Error::from_thread()],
        ))
    } else {
        Ok(Handle(handle))
    }
}

/// Downloads any HTTPS file, such as the What's New video.
pub fn download(url: &str, progress: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    http_get(url, progress)
}

/// HTTPS GET. GitHub's download links redirect to its file servers, which
/// WinHTTP follows on its own. `progress` gets (bytes so far, total if known).
#[cfg(windows)]
fn http_get(url: &str, mut progress: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("not an https URL: {url}"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = format!("/{path}");
    let agent = format!("RustCleanup/{}", env!("CARGO_PKG_VERSION"));

    // SAFETY: WinHTTP calls with valid, owned handles that `Handle` closes.
    unsafe {
        let session = open(
            WinHttpOpen(
                &HSTRING::from(agent.as_str()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                None,
                None,
                0,
            ),
            "session",
        )?;
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
        let connection = open(
            WinHttpConnect(
                session.0,
                &HSTRING::from(host),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ),
            "connect",
        )?;
        let request = open(
            WinHttpOpenRequest(
                connection.0,
                &HSTRING::from("GET"),
                &HSTRING::from(path.as_str()),
                None,
                None,
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "request",
        )?;
        let headers: Vec<u16> = "Accept: application/vnd.github+json, application/octet-stream\r\n"
            .encode_utf16()
            .collect();
        let network = |e: windows::core::Error| trf("Could not reach GitHub: {0}", &[&e.message()]);
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(network)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(network)?;

        let query_number = |level: u32| -> Option<u64> {
            let mut value = 0u32;
            let mut len = 4u32;
            WinHttpQueryHeaders(
                request.0,
                level | WINHTTP_QUERY_FLAG_NUMBER,
                None,
                Some(&mut value as *mut u32 as *mut _),
                &mut len,
                std::ptr::null_mut(),
            )
            .ok()
            .map(|_| value as u64)
        };
        let status = query_number(WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err(trf("GitHub answered with HTTP status {0}.", &[&status]));
        }
        // Content-Length (5); may be absent for chunked replies.
        let total = query_number(5);

        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
            )
            .map_err(network)?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            progress(body.len() as u64, total);
        }
        Ok(body)
    }
}

/// HTTPS GET through curl, following GitHub's redirects. `progress` gets
/// (bytes so far, total if known).
#[cfg(target_os = "linux")]
fn http_get(url: &str, mut progress: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    if !url.starts_with("https://") {
        return Err(format!("not an https URL: {url}"));
    }
    let curl = super::command::which("curl")
        .ok_or_else(|| trf("Could not reach GitHub: {0}", &[&"curl is not installed"]))?;
    let agent = format!("RustCleanup/{}", env!("CARGO_PKG_VERSION"));
    let mut child = Command::new(curl)
        .args(["-sSL", "--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "15", "--speed-time", "30", "--speed-limit", "1"])
        .args(["-A", &agent])
        .args(["-H", "Accept: application/vnd.github+json, application/octet-stream"])
        // Headers go to stderr, for the status and the length.
        .args(["-D", "/dev/stderr", "--", url])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| trf("Could not reach GitHub: {0}", &[&e]))?;

    let total = Arc::new(AtomicU64::new(0));
    let stderr = child.stderr.take();
    let header_total = total.clone();
    let headers = std::thread::spawn(move || {
        let (mut status, mut errors) = (0u64, String::new());
        for line in BufReader::new(stderr?).lines().map_while(Result::ok) {
            let line = line.trim_end();
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("http/") {
                status = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
                header_total.store(0, Ordering::Relaxed);
            } else if let Some(len) = lower.strip_prefix("content-length:") {
                header_total.store(len.trim().parse().unwrap_or(0), Ordering::Relaxed);
            } else if let Some(e) = line.strip_prefix("curl: ") {
                errors = e.to_string();
            }
        }
        Some((status, errors))
    });

    let mut body = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    if let Some(mut out) = child.stdout.take() {
        loop {
            match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    body.extend_from_slice(&buf[..n]);
                    let t = total.load(Ordering::Relaxed);
                    progress(body.len() as u64, (t > 0).then_some(t));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(trf("Could not reach GitHub: {0}", &[&e])),
            }
        }
    }
    let exit = child.wait().map_err(|e| trf("Could not reach GitHub: {0}", &[&e]))?;
    let (status, errors) = headers.join().ok().flatten().unwrap_or_default();
    if !exit.success() {
        let why = if errors.is_empty() { exit.to_string() } else { errors };
        return Err(trf("Could not reach GitHub: {0}", &[&why]));
    }
    if status != 200 {
        return Err(trf("GitHub answered with HTTP status {0}.", &[&status]));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v2.0.1", "2.0.0"));
        assert!(is_newer("2.1", "2.0.9"));
        assert!(is_newer("v10.0.0", "9.9.9"));
        assert!(!is_newer("v2.0.0", "2.0.0"));
        assert!(!is_newer("v1.9.9", "2.0.0"));
        assert!(!is_newer("garbage", "2.0.0"));
    }
}
