//! Root checks: the Linux side of "Run as administrator".

/// True when RustCleanup runs as root (effective uid 0).
pub fn is_elevated() -> bool {
    super::user::is_root()
}

/// True when this desktop can ask for root through polkit.
pub fn pkexec_available() -> bool {
    super::command::which("pkexec").is_some()
}

/// True under Windows Subsystem for Linux. Its apps have no login session,
/// so polkit (and with it `pkexec`) refuses every request there.
pub fn in_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some_and(|v| !v.is_empty())
        || std::path::Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}
