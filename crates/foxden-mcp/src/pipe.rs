//! The app's pipe: one connection per call, to a server run by this same user.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetLengthSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenMutexW, OpenProcessToken};

use crate::protocol::Relay;

pub const NOT_RUNNING: &str = "FoxDen isn't running. Start FoxDen and try again.";
pub const TURNED_OFF: &str = "Turn on AI assistant access in FoxDen's Settings.";
const BUSY: &str = "FoxDen is busy. Try again in a moment.";
const SILENT: &str = "FoxDen didn't answer in time.";
const CLOSED: &str = "FoxDen closed the connection.";
const STRANGER: &str = "The FoxDen pipe belongs to another account, so it was not used.";

const TIMEOUT: Duration = Duration::from_secs(20);
/// A fan proposal waits up to three minutes for the person at the PC, then for the service.
// Under the common 60 s client limit; the request stays on screen after this.
const APPROVAL: Duration = Duration::from_secs(50);
const PENDING: &str = "Still waiting: the person at this PC hasn't answered yet. The request stays on screen for up to 3 minutes. Call get_fan_config later and read last_proposal for the outcome.";
const PENDING_SETTINGS: &str = "Still waiting: the person at this PC hasn't answered yet. The request stays on screen for up to 3 minutes. Call get_settings later and read last_request for the outcome.";
/// How often a waiting call tells the client it is still going.
const TICK: Duration = Duration::from_secs(10);
const MAX_REPLY: u64 = 4 * 1024 * 1024;
const ERROR_PIPE_BUSY: i32 = 231;
/// Lets the server see who is calling without being able to act as the caller.
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;

pub struct AppPipe {
    name: Result<String, String>,
    app_running: fn() -> bool,
}

impl AppPipe {
    pub fn new() -> Self {
        let name = user_sid().map(|(_, sid)| format!(r"\\.\pipe\foxden-app-{sid}"));
        Self::with_name(name, app_running)
    }

    #[cfg(test)]
    pub fn with(name: String, app_running: fn() -> bool) -> Self {
        Self::with_name(Ok(name), app_running)
    }

    fn with_name(name: Result<String, String>, app_running: fn() -> bool) -> Self {
        AppPipe { name, app_running }
    }
}

impl Relay for AppPipe {
    fn call(&mut self, tool: &str, args: &Value, client: &str, waiting: &mut dyn FnMut()) -> Result<String, String> {
        let name = self.name.clone()?;
        let pipe = match open(&name) {
            Ok(pipe) => pipe,
            Err(Gone) if (self.app_running)() => return Err(TURNED_OFF.into()),
            Err(Gone) => return Err(NOT_RUNNING.into()),
            Err(Other(e)) => return Err(e),
        };
        let line = request(tool, args, client);
        let pending = match tool {
            "propose_fan_curves" => Some(PENDING),
            "set_settings" => Some(PENDING_SETTINGS),
            _ => None,
        };
        let timeout = if pending.is_some() { APPROVAL } else { TIMEOUT };
        let reply = match exchange(pipe, line, timeout, waiting) {
            Err(e) if e == SILENT && pending.is_some() => return Ok(pending.unwrap_or_default().into()),
            r => r?,
        };
        let reply: Value = serde_json::from_str(&reply)
            .map_err(|_| "FoxDen sent a reply that makes no sense.".to_string())?;
        match (reply.get("ok"), reply["error"].as_str()) {
            (_, Some(e)) => Err(e.to_string()),
            (Some(Value::String(text)), _) => Ok(text.clone()),
            (Some(v), _) => Ok(serde_json::to_string_pretty(v).unwrap_or_default()),
            (None, None) => Err("FoxDen sent a reply that makes no sense.".into()),
        }
    }
}

/// One request line; the app shows `client` when it asks the person at the PC.
fn request(tool: &str, args: &Value, client: &str) -> String {
    json!({"tool": tool, "args": args, "client": client}).to_string()
}

enum Failure {
    Gone,
    Other(String),
}
use Failure::{Gone, Other};

fn open(name: &str) -> Result<File, Failure> {
    for _ in 0..50 {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name);
        match opened {
            Ok(file) => {
                return if owned_by_me(&file) { Ok(file) } else { Err(Other(STRANGER.into())) };
            }
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(Gone),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(Other(format!("Cannot reach FoxDen: {e}"))),
        }
    }
    Err(Other(BUSY.into()))
}

/// Writes the request and waits a bounded time for the line that answers it.
fn exchange(pipe: File, line: String, timeout: Duration, waiting: &mut dyn FnMut()) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("foxden-mcp-call".into())
        .spawn(move || {
            let _ = tx.send(talk(&pipe, &line));
        })
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left.min(TICK)) {
            Ok(reply) => return reply,
            Err(mpsc::RecvTimeoutError::Timeout) if !left.is_zero() => waiting(),
            Err(_) => return Err(SILENT.to_string()),
        }
    }
}

fn talk(pipe: &File, line: &str) -> Result<String, String> {
    let closed = |_| CLOSED.to_string();
    (&*pipe).write_all(format!("{line}\n").as_bytes()).map_err(closed)?;
    let mut reply = Vec::new();
    BufReader::new(pipe).take(MAX_REPLY).read_until(b'\n', &mut reply).map_err(closed)?;
    match reply.pop() {
        Some(b'\n') => String::from_utf8(reply).map_err(|_| "FoxDen sent a reply that is not text.".into()),
        _ => Err(CLOSED.into()),
    }
}

/// Whether FoxDen holds its single-instance mutex in this session.
fn app_running() -> bool {
    let name: Vec<u16> = "Local\\FoxDenNative\0".encode_utf16().collect();
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

/// This process's user as a raw SID and as text.
fn user_sid() -> Result<(Vec<u8>, String), String> {
    let failed = || format!("Cannot read this account's id: {}", std::io::Error::last_os_error());
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(failed());
        }
        let mut len = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(failed());
        }
        let sid = (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let raw = std::slice::from_raw_parts(sid.cast::<u8>(), GetLengthSid(sid) as usize).to_vec();
        let mut text = null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err(failed());
        }
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        Ok((raw, s))
    }
}

fn owned_by_me(pipe: &File) -> bool {
    let Ok((mine, _)) = user_sid() else {
        return false;
    };
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
        return false;
    }
    let same = !owner.is_null() && unsafe { EqualSid(owner, mine.as_ptr().cast_mut().cast()) } != 0;
    unsafe { LocalFree(descriptor) };
    same
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_the_client() {
        let line: Value = serde_json::from_str(&request("propose_fan_curves", &json!({"reason": "r"}), "Zed")).unwrap();
        assert_eq!(line, json!({"tool": "propose_fan_curves", "args": {"reason": "r"}, "client": "Zed"}));
    }

    #[test]
    fn the_account_id_reads_as_a_sid() {
        let (raw, text) = user_sid().unwrap();
        assert!(text.starts_with("S-1-"), "{text}");
        assert!(raw.len() >= 8);
    }
}
