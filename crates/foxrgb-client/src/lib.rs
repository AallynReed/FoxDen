//! Client for the background service's named pipe.

#[cfg(test)]
mod tests;

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use foxrgb_fan::{FanConfig, FanStatus, SensorStatus, Status};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};

pub const PIPE: &str = r"\\.\pipe\foxrgb-service";

const NOT_RUNNING: &str = "the background service is not running";
const CLOSED: &str = "the service closed the connection";
const SILENT: &str = "the service is not answering";

const MAX_REPLY: u64 = 1024 * 1024;
const ERROR_PIPE_BUSY: i32 = 231;
/// Lets the server see who is calling without being able to act as the caller.
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetNamedPipeServerProcessId(pipe: RawHandle, pid: *mut u32) -> i32;
}

/// What the service says about itself and its hardware.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Report {
    pub version: String,
    /// Missing from a service older than the simulated fans switch.
    #[serde(default)]
    pub sim: bool,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Running,
    /// Nothing is listening on the pipe, which is a normal state.
    #[default]
    Stopped,
    /// It answered wrongly or the connection broke.
    Error,
}

/// The service and its fans as the app shows them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FanState {
    pub service: Service,
    /// What went wrong, when `service` is `Error`.
    pub detail: Option<String>,
    pub version: Option<String>,
    pub sim: bool,
    pub sensors: Vec<SensorStatus>,
    pub fans: Vec<FanStatus>,
    pub config: FanConfig,
}

#[derive(Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request<'a> {
    Status,
    GetConfig,
    SetConfig { config: &'a FanConfig },
    SetSim { on: bool },
    Shutdown,
}

/// The part every reply has.
#[derive(Deserialize)]
struct Verdict {
    ok: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ConfigReply {
    config: FanConfig,
}

type Reply = Result<String, String>;

struct Job {
    line: String,
    reply: Sender<Reply>,
}

struct Worker {
    jobs: Sender<Job>,
    /// The answer still owed to a call that ran out of time.
    late: Option<Receiver<Reply>>,
}

/// Talks to the service over one connection, opened when needed and again after it breaks.
pub struct Client {
    worker: Mutex<Worker>,
    timeout: Duration,
}

impl Client {
    /// No call waits longer than `timeout` for its answer, whatever the service does.
    pub fn new(pipe: &str, timeout: Duration) -> Client {
        let (jobs, queue) = mpsc::channel::<Job>();
        let pipe = pipe.to_string();
        thread::spawn(move || {
            let mut conn = None;
            for job in queue {
                let _ = job.reply.send(talk(&pipe, &mut conn, &job.line));
            }
        });
        Client {
            worker: Mutex::new(Worker { jobs, late: None }),
            timeout,
        }
    }

    pub fn status(&self) -> Result<Report, String> {
        self.ask(&Request::Status)
    }

    pub fn get_config(&self) -> Result<FanConfig, String> {
        Ok(self.ask::<ConfigReply>(&Request::GetConfig)?.config)
    }

    /// A config the service rejects comes back as the service's own error text.
    pub fn set_config(&self, config: &FanConfig) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::SetConfig { config })
            .map(drop)
    }

    /// Adds or removes the simulated sensors and fans.
    pub fn set_sim(&self, on: bool) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::SetSim { on }).map(drop)
    }

    /// Asks a service run from a console to release the fans and exit. An installed one refuses.
    pub fn shutdown(&self) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::Shutdown).map(drop)
    }

    /// Never fails: a service that is missing or misbehaving is a state of its own.
    pub fn state(&self) -> FanState {
        let asked = self
            .status()
            .and_then(|report| Ok((report, self.get_config()?)));
        match asked {
            Ok((report, config)) => FanState {
                service: Service::Running,
                detail: None,
                version: Some(report.version),
                sim: report.sim,
                sensors: report.status.sensors,
                fans: report.status.fans,
                config,
            },
            Err(e) if e == NOT_RUNNING => FanState::default(),
            Err(e) => FanState {
                service: Service::Error,
                detail: Some(e),
                ..FanState::default()
            },
        }
    }

    fn ask<T: DeserializeOwned>(&self, request: &Request) -> Result<T, String> {
        let line = self.exchange(serde_json::to_string(request).map_err(|e| e.to_string())?)?;
        let odd = |e| format!("the service sent a reply that makes no sense: {e}");
        let verdict: Verdict = serde_json::from_str(&line).map_err(odd)?;
        if !verdict.ok {
            return Err(verdict
                .error
                .unwrap_or_else(|| "the service refused the request".into()));
        }
        serde_json::from_str(&line).map_err(odd)
    }

    fn exchange(&self, line: String) -> Reply {
        let mut worker = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        let waiting = |late: &Receiver<Reply>| matches!(late.try_recv(), Err(TryRecvError::Empty));
        if worker.late.as_ref().is_some_and(waiting) {
            return Err(SILENT.into());
        }
        worker.late = None;

        let (reply, answer) = mpsc::channel();
        worker
            .jobs
            .send(Job { line, reply })
            .map_err(|_| SILENT.to_string())?;
        match answer.recv_timeout(self.timeout) {
            Ok(reply) => reply,
            Err(_) => {
                worker.late = Some(answer);
                Err(SILENT.into())
            }
        }
    }
}

/// The id of the process that serves `pipe`, if one does.
pub fn server_pid(pipe: &str) -> Option<u32> {
    let conn = connect(pipe).ok()?;
    let mut pid = 0;
    let found = unsafe { GetNamedPipeServerProcessId(conn.get_ref().as_raw_handle(), &mut pid) };
    (found != 0).then_some(pid)
}

type Conn = BufReader<File>;

fn talk(pipe: &str, conn: &mut Option<Conn>, line: &str) -> Reply {
    // A connection kept from an earlier call may lead to a service that has since gone.
    if let Some(kept) = conn {
        match send(kept, line) {
            Ok(reply) => return Ok(reply),
            Err(_) => *conn = None,
        }
    }
    let reply = send(conn.insert(connect(pipe)?), line);
    if reply.is_err() {
        *conn = None;
    }
    reply
}

fn connect(pipe: &str) -> Result<Conn, String> {
    // Busy means the server is between two instances of its pipe, which passes in a moment.
    for _ in 0..50 {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(pipe);
        match opened {
            Ok(file) => return Ok(BufReader::new(file)),
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(NOT_RUNNING.into()),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(format!("cannot reach the background service: {e}")),
        }
    }
    Err(SILENT.into())
}

/// Writes one request line and reads the line that answers it.
fn send(conn: &mut Conn, line: &str) -> Reply {
    let closed = |_| CLOSED.to_string();
    conn.get_ref()
        .write_all(format!("{line}\n").as_bytes())
        .map_err(closed)?;
    let mut reply = Vec::new();
    conn.by_ref()
        .take(MAX_REPLY)
        .read_until(b'\n', &mut reply)
        .map_err(closed)?;
    match reply.pop() {
        Some(b'\n') => {
            String::from_utf8(reply).map_err(|_| "the service sent a reply that is not text".into())
        }
        _ => Err(CLOSED.into()),
    }
}
