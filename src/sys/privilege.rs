//! Process token helpers: elevation checks, privileges, and a handle guard.

use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TokenElevation,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PCWSTR};

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
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| format!("OpenProcessToken: {e}"))?;
        let token = OwnedHandle(token);

        let mut luid = LUID::default();
        LookupPrivilegeValueW(PCWSTR::null(), &HSTRING::from(name), &mut luid)
            .map_err(|e| format!("LookupPrivilegeValue({name}): {e}"))?;

        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        AdjustTokenPrivileges(token.0, false, Some(&privileges), 0, None, None)
            .map_err(|e| format!("AdjustTokenPrivileges({name}): {e}"))?;
        // AdjustTokenPrivileges "succeeds" even when the account lacks the privilege.
        if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
            return Err(format!(
                "{name} is not held by this account (run as Administrator)"
            ));
        }
        Ok(())
    }
}
