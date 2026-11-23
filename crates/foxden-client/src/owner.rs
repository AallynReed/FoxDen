//! Who made the pipe, so a look-alike that someone else made is never talked to.

use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
use windows_sys::Win32::Security::{
    EqualSid, GetLengthSid, GetTokenInformation, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSID,
    TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser, WinBuiltinAdministratorsSid,
    WinLocalSystemSid,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// This process's user and the owner it gives what it makes, as raw SIDs.
fn own_sids() -> &'static [Vec<u8>] {
    static SIDS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    SIDS.get_or_init(|| unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for class in [TokenUser, TokenOwner] {
            let mut len = 0;
            GetTokenInformation(token, class, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            if GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &mut len) == 0 {
                continue;
            }
            // TOKEN_USER and TOKEN_OWNER both start with the SID pointer.
            let sid = if class == TokenUser {
                (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid
            } else {
                (*buf.as_ptr().cast::<TOKEN_OWNER>()).Owner
            };
            let n = GetLengthSid(sid) as usize;
            out.push(std::slice::from_raw_parts(sid.cast::<u8>(), n).to_vec());
        }
        CloseHandle(token);
        out
    })
}

/// SYSTEM, Administrators, or this process's own account.
pub(crate) fn trusted_owner(sid: PSID, own: &[Vec<u8>]) -> bool {
    unsafe {
        IsWellKnownSid(sid, WinLocalSystemSid) != 0
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
            || own
                .iter()
                .any(|o| EqualSid(sid, o.as_ptr().cast_mut().cast()) != 0)
    }
}

/// Refuses a pipe owned by anyone but SYSTEM, Administrators or this account.
pub(crate) fn verify(pipe: &File) -> Result<(), String> {
    let (mut owner, mut descriptor) = (null_mut(), null_mut());
    let failed = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle() as _,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if failed != 0 {
        let e = std::io::Error::from_raw_os_error(failed as i32);
        return Err(format!("cannot tell who runs the background service: {e}"));
    }
    let ok = !owner.is_null() && trusted_owner(owner, own_sids());
    unsafe { LocalFree(descriptor) };
    if ok {
        Ok(())
    } else {
        Err("the service pipe belongs to another account, so FoxDen does not use it".into())
    }
}
