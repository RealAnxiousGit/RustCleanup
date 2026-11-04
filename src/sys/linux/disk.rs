//! Mounted filesystems for the Scan Disk drop-down, read from
//! `/proc/self/mounts`.

use std::collections::HashSet;
use std::ffi::CString;
use std::path::Path;

use crate::sys::format::fmt_bytes;
use crate::i18n::n_;

/// A mounted filesystem.
#[derive(Clone, Debug)]
pub struct Drive {
    /// Mount point, e.g. `/` or `/media/usb`.
    pub root: String,
    /// The block device behind it, e.g. `/dev/sda2`.
    pub device: String,
    pub label: String,
    pub file_system: String,
    pub kind: &'static str,
    pub total_bytes: u64,
    /// Whether a checker exists for this filesystem.
    pub scannable: bool,
}

impl Drive {
    /// The mount point, the form the UI and the checkers use.
    pub fn volume(&self) -> &str {
        &self.root
    }

    /// Whether this filesystem can be checked and repaired while mounted
    /// (the "online scan" mode): Btrfs scrub and XFS online scrub.
    pub fn is_ntfs(&self) -> bool {
        matches!(self.file_system.as_str(), "btrfs" | "xfs")
    }

    pub fn display_name(&self) -> String {
        let label = if self.label.is_empty() {
            crate::i18n::tr(self.kind)
        } else {
            &self.label
        };
        let size = if self.total_bytes > 0 {
            format!(" · {}", fmt_bytes(self.total_bytes))
        } else {
            String::new()
        };
        let fs = if self.file_system.is_empty() {
            String::new()
        } else {
            format!(" · {}", self.file_system)
        };
        format!("{}  {label}{fs}{size}", self.root)
    }
}

/// Space on a filesystem right now.
#[derive(Clone, Copy, Debug)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

impl Space {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    pub fn fraction(&self) -> f32 {
        self.used() as f32 / self.total.max(1) as f32
    }
}

pub fn space(root: &str) -> Option<Space> {
    let path = CString::new(root).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid path and out-pointer.
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    let total = st.f_blocks as u64 * block;
    // `f_bavail` is what a normal user can still write, like `df`.
    let free = st.f_bavail as u64 * block;
    (total > 0).then_some(Space { total, free })
}

/// Filesystems that are checked by a tool, and the ones that are just shown.
const LOCAL: [&str; 11] = [
    "ext2", "ext3", "ext4", "btrfs", "xfs", "vfat", "msdos", "exfat", "f2fs", "ntfs", "ntfs3",
];
const NETWORK: [&str; 6] = ["nfs", "nfs4", "cifs", "smb3", "sshfs", "fuse.sshfs"];
const OTHER: [&str; 4] = ["zfs", "fuseblk", "iso9660", "udf"];

/// `\040` and friends in `/proc/mounts` back to characters.
fn unescape_mount(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 4], 8)
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The filesystem label of a device, from `/dev/disk/by-label`.
fn label_of(device: &str) -> String {
    let Ok(target) = std::fs::canonicalize(device) else {
        return String::new();
    };
    std::fs::read_dir("/dev/disk/by-label")
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| std::fs::canonicalize(e.path()).is_ok_and(|p| p == target))
        .map(|e| decode_hex_escapes(&e.file_name().to_string_lossy()))
        .unwrap_or_default()
}

/// udev writes spaces in labels as `\x20`.
fn decode_hex_escapes(s: &str) -> String {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && b.get(i + 1) == Some(&b'x')
            && i + 3 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16)
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True for USB sticks and card readers.
fn is_removable(device: &str) -> bool {
    let Ok(real) = std::fs::canonicalize(device) else {
        return false;
    };
    let Some(name) = real.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // A partition's `removable` flag lives on its parent disk.
    let class = Path::new("/sys/class/block").join(name);
    let disk = std::fs::canonicalize(&class)
        .ok()
        .filter(|p| p.join("partition").exists())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or(class);
    std::fs::read_to_string(disk.join("removable")).is_ok_and(|v| v.trim() == "1")
}

/// Lists every mounted filesystem worth showing, `/` first.
pub fn list_drives() -> Vec<Drive> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut seen = HashSet::new();
    let mut drives: Vec<Drive> = text
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let device = unescape_mount(cols.next()?);
            let root = unescape_mount(cols.next()?);
            let fs = cols.next()?.to_string();
            let (kind, scannable) = if LOCAL.contains(&fs.as_str()) {
                let removable = is_removable(&device);
                (
                    if removable { n_("Removable") } else { n_("Local Disk") },
                    device.starts_with("/dev/"),
                )
            } else if NETWORK.contains(&fs.as_str()) {
                (n_("Network"), false)
            } else if OTHER.contains(&fs.as_str()) {
                (
                    if fs == "iso9660" || fs == "udf" { n_("CD/DVD") } else { n_("Local Disk") },
                    false,
                )
            } else {
                return None;
            };
            // Snap packages and bind mounts show the same device again.
            if root.starts_with("/snap/") || root.starts_with("/var/snap/") || !seen.insert(device.clone()) {
                return None;
            }
            let total_bytes = space(&root).map_or(0, |s| s.total);
            Some(Drive {
                label: label_of(&device),
                root,
                device,
                file_system: fs,
                kind,
                total_bytes,
                scannable,
            })
        })
        .collect();
    drives.sort_by_key(|d| (d.root != "/", d.root.clone()));
    drives
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_mount_paths() {
        assert_eq!(unescape_mount("/media/My\\040Disk"), "/media/My Disk");
        assert_eq!(decode_hex_escapes("My\\x20Disk"), "My Disk");
    }

    #[test]
    fn finds_root() {
        let drives = list_drives();
        assert!(drives.first().is_some_and(|d| d.root == "/"), "{drives:?}");
        let s = space("/").expect("/ reports its space");
        assert!(s.free <= s.total && s.used() > 0);
    }
}
