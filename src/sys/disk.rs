//! Drive enumeration for the Scan Disk drop-down.

use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};
use windows::core::HSTRING;

use super::format::fmt_bytes;

/// A mounted drive letter.
#[derive(Clone, Debug)]
pub struct Drive {
    /// Root path, e.g. `C:\`.
    pub root: String,
    pub label: String,
    pub file_system: String,
    pub kind: &'static str,
    pub total_bytes: u64,
    /// Whether `chkdsk` can meaningfully run on this drive type.
    pub scannable: bool,
}

impl Drive {
    /// Drive letter with colon, e.g. `C:`, the form chkdsk expects.
    pub fn volume(&self) -> &str {
        self.root.trim_end_matches('\\')
    }

    /// chkdsk's `/scan` online mode only exists for NTFS.
    pub fn is_ntfs(&self) -> bool {
        self.file_system.eq_ignore_ascii_case("NTFS")
    }

    pub fn display_name(&self) -> String {
        let label = if self.label.is_empty() {
            self.kind
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

/// Lists every drive letter currently mounted.
pub fn list_drives() -> Vec<Drive> {
    // SAFETY: GetLogicalDrives has no parameters and cannot fail unsafely.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", (b'A' + i) as char);
            let wide = HSTRING::from(root.as_str());
            // SAFETY: `wide` is a valid NUL-terminated path for the duration of the call.
            let drive_type = unsafe { GetDriveTypeW(&wide) };
            let (kind, scannable) = match drive_type {
                2 => ("Removable", true),
                3 => ("Local Disk", true),
                4 => ("Network", false),
                5 => ("CD/DVD", false),
                6 => ("RAM Disk", true),
                _ => return None, // DRIVE_UNKNOWN / DRIVE_NO_ROOT_DIR
            };

            let mut label = [0u16; 261];
            let mut fs = [0u16; 261];
            // SAFETY: both buffers are writable and their lengths are passed via the slices.
            let has_volume = unsafe {
                GetVolumeInformationW(&wide, Some(&mut label), None, None, None, Some(&mut fs))
                    .is_ok()
            };
            let mut total = 0u64;
            // SAFETY: `total` is a valid out-pointer.
            let _ = unsafe { GetDiskFreeSpaceExW(&wide, None, Some(&mut total), None) };

            Some(Drive {
                root,
                label: if has_volume {
                    from_wide(&label)
                } else {
                    String::new()
                },
                file_system: if has_volume {
                    from_wide(&fs)
                } else {
                    String::new()
                },
                kind,
                total_bytes: total,
                // An empty card reader slot has no volume to check.
                scannable: scannable && has_volume,
            })
        })
        .collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_drive() {
        let drives = list_drives();
        assert!(
            drives
                .iter()
                .any(|d| d.volume().eq_ignore_ascii_case("C:") && d.scannable)
        );
    }
}
