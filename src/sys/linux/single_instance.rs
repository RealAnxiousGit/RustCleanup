//! Keeps a single copy of RustCleanup running per desktop user.
//!
//! The first copy holds an exclusive `flock` on a file in the user's runtime
//! folder and listens on a Unix socket next to it. A second copy (as the
//! user or as root) finds the lock taken, sends "show" over the socket and
//! exits. The tray module takes the listener and turns "show" into
//! "Open App".

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::user;

/// The listener, waiting for the tray to pick it up.
static LISTENER: Mutex<Option<UnixListener>> = Mutex::new(None);

/// Held for as long as this copy runs. Dropping it releases the lock.
pub struct InstanceLock {
    _file: File,
    socket: PathBuf,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        // Closing the file releases the flock.
    }
}

/// The desktop user's runtime folder, or a private folder in /tmp when
/// there is none.
fn dir() -> PathBuf {
    let runtime = user::desktop_user().runtime_dir();
    if runtime.is_dir() {
        return runtime;
    }
    let fallback = PathBuf::from(format!("/tmp/rustcleanup-{}", user::desktop_user().uid));
    if std::fs::metadata(&fallback).is_err() {
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&fallback);
        user::give_to_user(&fallback);
    }
    fallback
}

fn lock_path() -> PathBuf {
    dir().join("rustcleanup.lock")
}

fn socket_path() -> PathBuf {
    dir().join("rustcleanup.sock")
}

fn try_lock() -> Option<InstanceLock> {
    let path = lock_path();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .ok()?;
    user::give_to_user(&path);
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return None;
    }
    let socket = socket_path();
    // We hold the lock, so any socket file left over is stale.
    let _ = std::fs::remove_file(&socket);
    if let Ok(listener) = UnixListener::bind(&socket) {
        user::give_to_user(&socket);
        if let Ok(mut slot) = LISTENER.lock() {
            *slot = Some(listener);
        }
    }
    Some(InstanceLock { _file: file, socket })
}

/// The "show yourself" listener, for the tray thread. `None` after the
/// first call or if the socket could not be created.
pub fn take_listener() -> Option<UnixListener> {
    LISTENER.lock().ok()?.take()
}

/// Becomes the only running copy, or asks the running copy to show itself
/// and returns `None` (the caller then exits).
///
/// `wait` is used after an update or restart, while the previous copy is
/// still shutting down: it keeps trying for a few seconds instead of handing over.
pub fn acquire(wait: bool) -> Option<InstanceLock> {
    let deadline = Instant::now() + Duration::from_secs(if wait { 15 } else { 0 });
    loop {
        if let Some(lock) = try_lock() {
            return Some(lock);
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    show_running_copy();
    None
}

/// Asks the running copy to quit, for the root copy that replaces it.
/// True if one was there to ask.
pub fn ask_running_copy_to_quit() -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(b"quit\n").is_ok(),
        Err(_) => false,
    }
}

/// Sends "show" to the running copy.
fn show_running_copy() {
    // The first copy may still be starting up, so try a few times.
    for _ in 0..25 {
        if let Ok(mut stream) = UnixStream::connect(socket_path()) {
            let _ = stream.write_all(b"show\n");
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
