use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::host::{Host, MAX_LINE, log};

pub const PIPE_NAME: &str = r"\\.\pipe\foxrgb-service";
const MAX_CLIENTS: usize = 16;

/// Creates one unconnected instance of the pipe. `first` fails if the name is already taken.
pub fn create(name: &str, first: bool) -> Result<File, String> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let first_flag = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | first_flag,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            MAX_LINE as u32,
            MAX_LINE as u32,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let e = std::io::Error::last_os_error();
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
    let clients = Arc::new(AtomicUsize::new(0));
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
        if clients.fetch_add(1, Ordering::Relaxed) >= MAX_CLIENTS {
            clients.fetch_sub(1, Ordering::Relaxed);
            continue;
        }
        let (host, clients) = (host.clone(), clients.clone());
        thread::spawn(move || {
            serve_client(&host, &pipe);
            clients.fetch_sub(1, Ordering::Relaxed);
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
        let mut reply = host.handle(line.trim_end_matches(['\r', '\n']));
        reply.push('\n');
        // An unterminated line is either too long or cut off; neither can be followed.
        if writer.write_all(reply.as_bytes()).is_err() || !complete {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use foxrgb_fan::SimBackend;
    use serde_json::{Value, json};

    use super::*;
    use crate::host::tests::{sample, scratch, wait_for};
    use crate::host::{load_config, start};

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

    fn ask(pipe: &File, request: &str) -> Value {
        let mut writer = pipe;
        writer.write_all(format!("{request}\n").as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(pipe).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn serves_status_and_set_config_over_the_pipe() {
        let dir = scratch("pipe");
        let name = format!(r"\\.\pipe\foxrgb-service-test-{}", std::process::id());
        let running = start(dir.clone(), vec![Box::<SimBackend>::default()], &name).unwrap();
        assert!(create(&name, true).is_err());

        let silent = open(&name);
        let pipe = open(&name);
        let status = ask(&pipe, r#"{"cmd":"status"}"#);
        assert_eq!(status["ok"], true);
        assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
        wait_for("simulated fans in the status", || {
            let fans = &ask(&pipe, r#"{"cmd":"status"}"#)["status"]["fans"];
            fans.as_array().is_some_and(|fans| !fans.is_empty())
        });

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
}
