//! Root checks: the Linux side of "Run as administrator".

/// True when RustCleanup runs as root (effective uid 0).
pub fn is_elevated() -> bool {
    super::user::is_root()
}

/// True when this desktop can ask for root through polkit.
pub fn pkexec_available() -> bool {
    super::command::which("pkexec").is_some()
}
