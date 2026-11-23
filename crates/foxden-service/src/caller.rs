//! Who is on the other end of the pipe, and whether they may change anything.

use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, EqualSid, GetLengthSid, GetTokenInformation,
    IsWellKnownSid, RevertToSelf, SECURITY_MAX_SID_SIZE, TOKEN_INFORMATION_CLASS, TOKEN_QUERY,
    TOKEN_USER, TokenSessionId, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows_sys::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW, WTSActive, WTSEnumerateSessionsW, WTSFreeMemory,
    WTSGetActiveConsoleSessionId, WTSQuerySessionInformationW, WTSUserName,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

pub const NOT_ALLOWED: &str = "only the person signed in at this PC's screen, or an administrator, \
     can change this; from another Windows session FoxDen can only read the fans";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caller {
    /// The Windows session the caller runs in.
    pub session: Option<u32>,
    /// An elevated administrator, or SYSTEM.
    pub admin: bool,
    /// The same account as this process, such as the app with a service it started itself.
    pub same_user: bool,
}

/// Who is at the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Console {
    pub session: Option<u32>,
    /// Someone is signed in there.
    pub occupied: bool,
    /// Sessions with someone signed in and connected, looked up only when the console is free.
    pub active: Vec<u32>,
}

impl Caller {
    /// Read only: a caller whose token could not be read.
    pub const NOBODY: Caller = Caller {
        session: None,
        admin: false,
        same_user: false,
    };

    /// May change settings and hardware.
    pub fn may_change(&self) -> bool {
        self.admin || self.same_user || self.session.is_some_and(|s| at_console(s, &console()))
    }

    /// The client of `pipe` after its first line; `Err` means this thread must end.
    pub fn of_pipe(pipe: &File) -> Result<Caller, ()> {
        unsafe {
            if ImpersonateNamedPipeClient(pipe.as_raw_handle() as _) == 0 {
                return Ok(Caller::NOBODY);
            }
            let mut token: HANDLE = null_mut();
            let opened = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) != 0;
            if RevertToSelf() == 0 {
                if opened {
                    CloseHandle(token);
                }
                return Err(());
            }
            if !opened {
                return Ok(Caller::NOBODY);
            }
            let caller = of_token(token);
            CloseHandle(token);
            Ok(caller)
        }
    }
}

/// At the screen, or the only session in use while nobody is at the screen (Remote Desktop).
pub fn at_console(session: u32, console: &Console) -> bool {
    console.session == Some(session) || (!console.occupied && console.active == [session])
}

/// Who is at the screen, cached for a second.
fn console() -> Console {
    static LAST: Mutex<Option<(Instant, Console)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    match &*last {
        Some((at, known)) if at.elapsed() < Duration::from_secs(1) => known.clone(),
        _ => {
            let now = look_up();
            *last = Some((Instant::now(), now.clone()));
            now
        }
    }
}

fn look_up() -> Console {
    let id = unsafe { WTSGetActiveConsoleSessionId() };
    let session = (id != u32::MAX).then_some(id);
    let occupied = session.is_some_and(signed_in);
    let active = if occupied { Vec::new() } else { active() };
    Console {
        session,
        occupied,
        active,
    }
}

fn signed_in(session: u32) -> bool {
    unsafe {
        let (mut text, mut len) = (null_mut(), 0);
        if WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            WTSUserName,
            &mut text,
            &mut len,
        ) == 0
        {
            return false;
        }
        let named = !text.is_null()
            && std::slice::from_raw_parts(text, len as usize / 2).first().is_some_and(|&c| c != 0);
        WTSFreeMemory(text.cast());
        named
    }
}

fn active() -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let (mut list, mut count): (*mut WTS_SESSION_INFOW, u32) = (null_mut(), 0);
        if WTSEnumerateSessionsW(WTS_CURRENT_SERVER_HANDLE, 0, 1, &mut list, &mut count) == 0 {
            return out;
        }
        for info in std::slice::from_raw_parts(list, count as usize) {
            if info.State == WTSActive && signed_in(info.SessionId) {
                out.push(info.SessionId);
            }
        }
        WTSFreeMemory(list.cast());
    }
    out
}

unsafe fn info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    let mut len = 0;
    unsafe {
        GetTokenInformation(token, class, null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        (GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &mut len) != 0)
            .then_some(buf)
    }
}

unsafe fn user_sid(token: HANDLE) -> Option<Vec<u8>> {
    unsafe {
        let buf = info(token, TokenUser)?;
        let sid = (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let n = GetLengthSid(sid) as usize;
        Some(std::slice::from_raw_parts(sid.cast::<u8>(), n).to_vec())
    }
}

fn own_sid() -> Option<&'static [u8]> {
    static OWN: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    OWN.get_or_init(|| unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let sid = user_sid(token);
        CloseHandle(token);
        sid
    })
    .as_deref()
}

unsafe fn of_token(token: HANDLE) -> Caller {
    unsafe {
        let session = info(token, TokenSessionId).map(|b| b[0] as u32);
        let user = user_sid(token);
        let system = user
            .as_ref()
            .is_some_and(|u| IsWellKnownSid(u.as_ptr().cast_mut().cast(), WinLocalSystemSid) != 0);
        let mut admins = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = admins.len() as u32;
        let mut member = 0;
        let admin = system
            || (CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                admins.as_mut_ptr().cast(),
                &mut size,
            ) != 0
                && CheckTokenMembership(token, admins.as_mut_ptr().cast(), &mut member) != 0
                && member != 0);
        let same_user = match (user, own_sid()) {
            (Some(user), Some(own)) => {
                EqualSid(
                    user.as_ptr().cast_mut().cast(),
                    own.as_ptr().cast_mut().cast(),
                ) != 0
            }
            _ => false,
        };
        Caller {
            session,
            admin,
            same_user,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console(session: Option<u32>, occupied: bool, active: &[u32]) -> Console {
        Console {
            session,
            occupied,
            active: active.to_vec(),
        }
    }

    #[test]
    fn the_screen_or_the_only_session_in_use_may_change_things() {
        assert!(at_console(1, &console(Some(1), true, &[])));
        assert!(!at_console(2, &console(Some(1), true, &[])));
        // Remote Desktop only: nobody at the screen, one session in use.
        assert!(at_console(3, &console(Some(1), false, &[3])));
        assert!(at_console(3, &console(None, false, &[3])));
        assert!(!at_console(3, &console(Some(1), false, &[2, 3])));
        assert!(!at_console(3, &console(None, false, &[])));
    }

    fn own_session() -> u32 {
        let mut session = 0;
        let ok = unsafe {
            windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(
                std::process::id(),
                &mut session,
            )
        };
        assert_ne!(ok, 0);
        session
    }

    #[test]
    fn a_client_that_only_lets_itself_be_identified_is_still_known() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::windows::fs::OpenOptionsExt;

        let name = crate::host::tests::pipe_name("caller");
        let server = crate::pipe::create(&name, true).unwrap();
        let client = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(0x0001_0000)
            .open(&name)
            .unwrap();
        assert!(crate::pipe::connect(&server));
        (&client).write_all(b"hello\n").unwrap();
        let mut line = String::new();
        BufReader::new(&server).read_line(&mut line).unwrap();
        let caller = Caller::of_pipe(&server).unwrap();
        assert_eq!(caller.session, Some(own_session()));
        assert!(caller.same_user);
    }

    #[test]
    #[ignore = "needs a desktop session at this PC's screen"]
    fn the_person_at_the_screen_may_change_things() {
        let caller = Caller {
            session: Some(own_session()),
            admin: false,
            same_user: false,
        };
        assert!(caller.may_change());
    }

    #[test]
    fn an_administrator_or_the_same_account_may_change_things_from_anywhere() {
        let far = Some(u32::MAX - 1);
        let admin = Caller {
            session: far,
            admin: true,
            same_user: false,
        };
        let same = Caller {
            admin: false,
            same_user: true,
            ..admin
        };
        let other = Caller {
            same_user: false,
            ..same
        };
        assert!(admin.may_change() && same.may_change());
        assert!(!other.may_change());
        assert!(!Caller::NOBODY.may_change());
    }
}
