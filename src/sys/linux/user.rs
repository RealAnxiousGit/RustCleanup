//! The person at the desktop, even while RustCleanup runs as root.
//!
//! Run through `pkexec` or `sudo`, RustCleanup is root, but links, file
//! managers, settings and the autostart entry all belong to the user who
//! started it. `pkexec` leaves their uid in `PKEXEC_UID` and `sudo` in
//! `SUDO_UID`.

use std::ffi::{CStr, CString};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: PathBuf,
}

impl User {
    /// `/run/user/<uid>`, where the session's sockets live.
    pub fn runtime_dir(&self) -> PathBuf {
        if !is_root()
            && let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
        {
            return dir.into();
        }
        PathBuf::from(format!("/run/user/{}", self.uid))
    }
}

pub fn is_root() -> bool {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::geteuid() == 0 }
}

fn lookup(uid: u32) -> Option<User> {
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call and `buf` outlives `pwd`'s strings.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r filled these with NUL-terminated strings inside `buf`.
    let (name, home) = unsafe {
        (
            CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned(),
            CStr::from_ptr(pwd.pw_dir).to_string_lossy().into_owned(),
        )
    };
    Some(User {
        uid,
        gid: pwd.pw_gid,
        name,
        home: home.into(),
    })
}

/// The desktop user: whoever started RustCleanup, before pkexec or sudo.
pub fn desktop_user() -> &'static User {
    static USER: OnceLock<User> = OnceLock::new();
    USER.get_or_init(|| {
        // SAFETY: no arguments, cannot fail.
        let uid = unsafe { libc::getuid() };
        let original = ["PKEXEC_UID", "SUDO_UID"]
            .iter()
            .find_map(|v| std::env::var(v).ok()?.parse::<u32>().ok());
        let uid = if uid == 0 { original.unwrap_or(0) } else { uid };
        lookup(uid).unwrap_or_else(|| User {
            uid,
            gid: uid,
            name: std::env::var("USER").unwrap_or_default(),
            home: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default(),
        })
    })
}

/// True when RustCleanup is root on behalf of a normal desktop user.
pub fn acting_for_user() -> bool {
    is_root() && desktop_user().uid != 0
}

/// `~/.config` of the desktop user.
pub fn config_home() -> PathBuf {
    if !acting_for_user()
        && let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty())
    {
        return dir.into();
    }
    desktop_user().home.join(".config")
}

/// Hands a file or folder RustCleanup created as root to the desktop user,
/// so their own programs can still change it.
pub fn give_to_user(path: &Path) {
    if !acting_for_user() {
        return;
    }
    let user = desktop_user();
    if let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) {
        // SAFETY: a valid path string; a failure only leaves root as owner.
        unsafe {
            libc::chown(c.as_ptr(), user.uid, user.gid);
        }
    }
}

/// Creates `dir` and every missing parent, owned by the desktop user.
pub fn create_dir(dir: &Path) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut at = Some(dir);
    while let Some(d) = at.filter(|d| !d.exists()) {
        missing.push(d.to_path_buf());
        at = d.parent();
    }
    std::fs::create_dir_all(dir)?;
    for d in missing {
        give_to_user(&d);
    }
    Ok(())
}

/// A command that runs as the desktop user, never as root: browsers, file
/// managers and settings apps must not get RustCleanup's rights.
pub fn user_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    if acting_for_user() {
        let user = desktop_user();
        let runtime = user.runtime_dir();
        cmd.env("HOME", &user.home)
            .env("USER", &user.name)
            .env("LOGNAME", &user.name)
            .env("XDG_RUNTIME_DIR", &runtime)
            // The same bus RustCleanup uses (`prepare_session_env` fills in
            // the user's default one when sudo or pkexec dropped it).
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                std::env::var_os("DBUS_SESSION_BUS_ADDRESS").unwrap_or_else(|| {
                    format!("unix:path={}", runtime.join("bus").display()).into()
                }),
            )
            .env_remove("PKEXEC_UID")
            .env_remove("SUDO_UID")
            .env_remove("SUDO_USER")
            .env_remove("SUDO_COMMAND");
        let name = CString::new(user.name.clone()).unwrap_or_default();
        let (uid, gid) = (user.uid, user.gid);
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                if libc::initgroups(name.as_ptr(), gid) != 0
                    || libc::setgid(gid) != 0
                    || libc::setuid(uid) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd
}

/// Runs `f` with the desktop user's effective user and group, then switches
/// back to root. Used for the tray, because the user's session bus does not
/// let root connect. The glibc calls change every thread at once, which
/// matters because zbus connects and authenticates on its own helper threads.
/// The switch lasts only while the tray connects at startup; the connection
/// stays open after RustCleanup is root again.
pub fn with_user_identity<T>(f: impl FnOnce() -> T) -> T {
    if !acting_for_user() {
        return f();
    }
    let (uid, gid) = (desktop_user().uid, desktop_user().gid);
    let keep = libc::uid_t::MAX; // -1: leave the real and saved ids (root)
    // SAFETY: plain credential calls; the saved ids stay root, so the
    // switch back below always succeeds.
    let switched = unsafe {
        libc::setresgid(keep, gid, keep) == 0 && libc::setresuid(keep, uid, keep) == 0
    };
    let out = f();
    if switched {
        // SAFETY: as above, back to root for the whole process.
        unsafe {
            libc::setresuid(keep, 0, keep);
            libc::setresgid(keep, 0, keep);
        }
    }
    out
}

/// Fills in the session variables `sudo` drops, so a root copy can still
/// reach the desktop's display, D-Bus (tray and notifications) and sound.
/// Called first thing in `main`, before any other thread exists.
pub fn prepare_session_env() {
    if !acting_for_user() {
        return;
    }
    let runtime = desktop_user().runtime_dir();
    let set = |name: &str, value: &std::ffi::OsStr| {
        if std::env::var_os(name).is_none_or(|v| v.is_empty()) {
            // SAFETY: called from `main` before any thread is started.
            unsafe { std::env::set_var(name, value) };
        }
    };
    set("XDG_RUNTIME_DIR", runtime.as_os_str());
    let bus = format!("unix:path={}", runtime.join("bus").display());
    set("DBUS_SESSION_BUS_ADDRESS", bus.as_ref());
    if std::env::var_os("DISPLAY").is_none() && runtime.join("wayland-0").exists() {
        set("WAYLAND_DISPLAY", "wayland-0".as_ref());
    }
}
