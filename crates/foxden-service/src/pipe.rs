use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::host::{Host, MAX_LINE, log};

pub const PIPE_NAME: &str = r"\\.\pipe\foxden-service";
const MAX_CLIENTS: usize = 16;
/// Who may open the pipe. By default users could only read one made by LocalSystem.
/// 0x12019b is read and write without the right to add instances.
const ACCESS: &str =
    "D:P(D;;FA;;;NU)(D;;FA;;;AN)(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)(A;;0x12019b;;;IU)";

/// Creates one unconnected instance of the pipe. `first` fails if the name is already taken.
pub fn create(name: &str, first: bool) -> Result<File, String> {
    create_with(name, first, ACCESS)
}

fn create_with(name: &str, first: bool, access: &str) -> Result<File, String> {
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let (name_w, access_w) = (wide(name), wide(access));
    let mut descriptor = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            access_w.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        let e = std::io::Error::last_os_error();
        return Err(format!("cannot build the access list of pipe {name}: {e}"));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let first_flag = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    let handle = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            PIPE_ACCESS_DUPLEX | first_flag,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            MAX_LINE as u32,
            MAX_LINE as u32,
            0,
            &attributes,
        )
    };
    let e = std::io::Error::last_os_error();
    unsafe { LocalFree(descriptor) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!("cannot create pipe {name}: {e}"));
    }
    Ok(unsafe { File::from_raw_handle(handle as _) })
}

fn connect(pipe: &File) -> bool {
    let ok = unsafe { ConnectNamedPipe(pipe.as_raw_handle() as _, std::ptr::null_mut()) };
    ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED
}

/// Unblocks `serve` by connecting to it as a client.
pub fn wake(name: &str) {
    let _ = OpenOptions::new().read(true).write(true).open(name);
}

/// Accepts clients until the host stops, one thread per client.
pub fn serve(host: &Arc<Host>, name: &str, first: File) {
    // Every client thread holds a clone, which it gives back even when it panics.
    let slots = Arc::new(());
    let mut listener = first;
    while !host.stopping() {
        let connected = connect(&listener);
        if host.stopping() {
            return;
        }
        // Created before this one is handed off, so the name is never free to take.
        let next = match create(name, false) {
            Ok(next) => next,
            Err(e) => {
                log(&host.dir, &e);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let pipe = std::mem::replace(&mut listener, next);
        if !connected {
            continue;
        }
        if Arc::strong_count(&slots) > MAX_CLIENTS {
            continue;
        }
        let (host, slot) = (host.clone(), slots.clone());
        thread::spawn(move || {
            let _slot = slot;
            serve_client(&host, &pipe);
        });
    }
}

fn serve_client(host: &Host, pipe: &File) {
    let mut reader = BufReader::new(pipe);
    let mut writer = pipe;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader
            .by_ref()
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(n) if n > 0 => {}
            _ => return,
        }
        let complete = buf.ends_with(b"\n");
        let line = String::from_utf8_lossy(&buf);
        let (mut reply, shutdown) = host.handle(line.trim_end_matches(['\r', '\n']));
        reply.push('\n');
        let sent = writer.write_all(reply.as_bytes()).is_ok();
        // Only now, so the reply is in the pipe before the process starts to stop.
        if shutdown {
            host.end(Ok(()));
        }
        // An unterminated line is either too long or cut off; neither can be followed.
        if !sent || !complete {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    use super::*;
    use crate::host::tests::{pipe_name, sample, scratch, wait_for};
    use crate::host::{Mode, Running, load_config, start};

    fn open(name: &str) -> File {
        loop {
            match OpenOptions::new().read(true).write(true).open(name) {
                Ok(pipe) => return pipe,
                // ERROR_PIPE_BUSY: the server is between two instances.
                Err(e) if e.raw_os_error() == Some(231) => thread::sleep(Duration::from_millis(10)),
                Err(e) => panic!("{name}: {e}"),
            }
        }
    }

    /// The reply to `request`, or `None` when the server hangs up instead.
    fn try_ask(pipe: &File, request: &str) -> Option<Value> {
        let mut writer = pipe;
        writer.write_all(format!("{request}\n").as_bytes()).ok()?;
        let mut line = String::new();
        BufReader::new(pipe).read_line(&mut line).ok()?;
        serde_json::from_str(&line).ok()
    }

    fn ask(pipe: &File, request: &str) -> Value {
        try_ask(pipe, request).expect("a reply")
    }

    /// A host with no hardware, the name of its pipe and its data directory.
    fn bare_host(name: &str, mode: Mode) -> (Running, String, PathBuf) {
        let (pipe, dir) = (pipe_name(name), scratch(name));
        let running = start(dir.clone(), Vec::new, mode, &pipe).unwrap();
        (running, pipe, dir)
    }

    /// The pipe's access list as the system reports it.
    fn access_of(pipe: &File) -> String {
        let mut descriptor = std::ptr::null_mut();
        let failed = unsafe {
            GetSecurityInfo(
                pipe.as_raw_handle() as _,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(failed, 0);
        let (mut text, mut len) = (std::ptr::null_mut(), 0);
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                &mut len,
            )
        };
        assert_ne!(converted, 0);
        let units = unsafe { std::slice::from_raw_parts(text, len as usize) };
        let access = String::from_utf16_lossy(units);
        unsafe {
            LocalFree(text.cast());
            LocalFree(descriptor);
        }
        access.trim_end_matches('\0').to_string()
    }

    #[test]
    fn serves_the_protocol_over_the_pipe() {
        let (running, name, dir) = bare_host("pipe", Mode::Console);
        assert!(create(&name, true).is_err());

        let silent = open(&name);
        let pipe = open(&name);
        let status = ask(&pipe, r#"{"cmd":"status"}"#);
        assert_eq!(status["ok"], true);
        assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(status["status"]["fans"], json!([]));

        let set = json!({"cmd": "set_config", "config": sample()}).to_string();
        assert_eq!(ask(&pipe, &set), json!({"ok": true}));
        assert_eq!(
            ask(&pipe, r#"{"cmd":"get_config"}"#)["config"],
            json!(sample())
        );
        assert_eq!(load_config(&dir), Ok(sample()));
        assert_eq!(ask(&pipe, "nonsense")["ok"], false);

        let long = "x".repeat(MAX_LINE + 10);
        assert_eq!(ask(&open(&name), &long)["ok"], false);

        drop(silent);
        running.stop();
        let log = std::fs::read_to_string(dir.join("service.log")).unwrap();
        assert!(log.contains("released"), "{log}");
    }

    #[test]
    fn shutdown_over_the_pipe_ends_a_console_host() {
        let (running, name, _dir) = bare_host("shutdown-console", Mode::Console);
        let pipe = open(&name);
        assert_eq!(running.ended(), None);
        assert_eq!(ask(&pipe, r#"{"cmd":"shutdown"}"#), json!({"ok": true}));
        wait_for("the request to end", || running.ended() == Some(Ok(())));
    }

    #[test]
    fn shutdown_over_the_pipe_is_refused_by_a_service() {
        let (running, name, _dir) = bare_host("shutdown-service", Mode::Service);
        let pipe = open(&name);
        assert_eq!(ask(&pipe, r#"{"cmd":"shutdown"}"#)["ok"], false);
        assert_eq!(ask(&pipe, r#"{"cmd":"status"}"#)["ok"], true);
        assert_eq!(running.ended(), None);
    }

    #[test]
    fn clients_beyond_the_limit_are_turned_away() {
        let (_running, name, _dir) = bare_host("limit", Mode::Console);
        let status = r#"{"cmd":"status"}"#;
        let mut idle: Vec<File> = (0..MAX_CLIENTS).map(|_| open(&name)).collect();
        assert_eq!(try_ask(&open(&name), status), None);
        assert_eq!(ask(&idle[0], status)["ok"], true);

        // A place that comes free is given out again.
        idle.pop();
        wait_for("a free place", || try_ask(&open(&name), status).is_some());
    }

    #[test]
    fn a_client_thread_that_panics_gives_its_place_back() {
        let (_running, name, _dir) = bare_host("panic-client", Mode::Console);
        for _ in 0..=MAX_CLIENTS {
            assert_eq!(try_ask(&open(&name), r#"{"cmd":"panic"}"#), None);
        }
        assert_eq!(ask(&open(&name), r#"{"cmd":"status"}"#)["ok"], true);
    }

    #[test]
    fn the_pipe_carries_its_access_list() {
        let name = pipe_name("access");
        let first = create(&name, true).unwrap();
        assert_eq!(access_of(&first), ACCESS);
        // Later instances share the first one's list and the owner may still add them.
        let second = create(&name, false).unwrap();
        assert_eq!(access_of(&second), ACCESS);
    }

    #[test]
    fn a_client_of_the_same_user_still_connects() {
        let name = pipe_name("same-user");
        let server = create(&name, true).unwrap();
        let client = open(&name);
        assert!(connect(&server));

        (&client).write_all(b"ping\n").unwrap();
        let mut line = String::new();
        BufReader::new(&server).read_line(&mut line).unwrap();
        assert_eq!(line, "ping\n");
        (&server).write_all(b"pong\n").unwrap();
        line.clear();
        BufReader::new(&client).read_line(&mut line).unwrap();
        assert_eq!(line, "pong\n");
    }

    #[test]
    fn the_rights_of_an_interactive_user_do_not_add_instances() {
        // The owner stands in for the interactive user here: it gets those rights and no others.
        let name = pipe_name("rights");
        let server = create_with(&name, true, "D:P(A;;0x12019b;;;OW)").unwrap();
        let client = open(&name);
        assert!(connect(&server));
        (&client).write_all(b"ping\n").unwrap();
        let mut line = String::new();
        BufReader::new(&server).read_line(&mut line).unwrap();
        assert_eq!(line, "ping\n");

        let refused = create_with(&name, false, ACCESS).unwrap_err();
        assert!(refused.contains("os error 5"), "{refused}");
    }
}
