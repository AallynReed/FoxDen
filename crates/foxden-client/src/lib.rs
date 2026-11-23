//! Client for the background service's named pipe.

mod owner;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use foxden_core::conflicts::{Closed, Scope};
use foxden_fan::{FanConfig, FanStatus, SensorStatus, Status};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};

pub const PIPE: &str = r"\\.\pipe\foxden-service";

const NOT_RUNNING: &str = "the background service is not running";
const CLOSED: &str = "the service closed the connection";
const SILENT: &str = "the service is not answering";

const MAX_REPLY: u64 = 1024 * 1024;
/// Below the service's 64 KiB line limit, with room for the request around the frames.
const MAX_REQUEST: usize = 60 * 1024;
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
    pub sensors: Vec<SensorStatus>,
    pub fans: Vec<FanStatus>,
    pub config: FanConfig,
}

#[derive(Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request<'a> {
    Status,
    GetConfig,
    SetConfig {
        config: &'a FanConfig,
    },
    Shutdown,
    Lights,
    SetLights {
        device: &'a str,
        colors: &'a [foxden_core::Color],
    },
    CloseConflicts(Scope),
    HubLights {
        lengths: &'a BTreeMap<String, usize>,
    },
    SetHubLights {
        frames: &'a [HubFrame],
    },
    ReleaseHubLights {
        generation: u64,
    },
    SetHubHardware {
        device: &'a str,
        setting: Option<&'a foxden_core::HardwareSetting>,
    },
    VirtualMic {
        action: &'a str,
    },
    SmbusReport,
}

/// One device's colours in a `set_hub_lights` batch, as `rrggbb` hex.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HubFrame {
    pub device: String,
    pub colors: String,
}

/// A hub zone whose LED count the user sets, as the service opened it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubZone {
    pub device: String,
    pub index: usize,
    pub name: String,
    pub leds: usize,
    pub min: usize,
    pub max: usize,
    /// LEDs per fan; 0 or 1 for a free count.
    #[serde(default)]
    pub step: usize,
}

/// The fan hubs whose lighting the service drives.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct HubLights {
    pub devices: Vec<foxden_core::Device>,
    pub zones: Vec<HubZone>,
    /// Names this opening, so a late release cannot close a newer one.
    #[serde(default)]
    pub generation: u64,
    /// Effects of their own, by device; empty from older services.
    #[serde(default)]
    pub hardware: BTreeMap<String, foxden_core::Hardware>,
}

/// What became of a frame: devices that failed, and whether the hubs were open at all.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct HubSent {
    #[serde(default = "open")]
    pub open: bool,
    #[serde(default)]
    pub failed: BTreeMap<String, String>,
}

fn open() -> bool {
    true
}

#[derive(Deserialize)]
struct LightsReply {
    devices: Vec<foxden_core::Device>,
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
    pipe: String,
    worker: Mutex<Worker>,
    timeout: Duration,
}

impl Client {
    /// No call waits longer than `timeout` for its answer, whatever the service does.
    pub fn new(pipe: &str, timeout: Duration) -> Client {
        let (jobs, queue) = mpsc::channel::<Job>();
        let name = pipe.to_string();
        thread::spawn(move || {
            let mut conn = None;
            for job in queue {
                let _ = job.reply.send(talk(&name, &mut conn, &job.line));
            }
        });
        Client {
            pipe: pipe.to_string(),
            worker: Mutex::new(Worker { jobs, late: None }),
            timeout,
        }
    }

    pub fn status(&self) -> Result<Report, String> {
        self.ask(&Request::Status)
    }

    /// Lighting devices only the service can reach, such as memory on the SMBus.
    pub fn lights(&self) -> Result<Vec<foxden_core::Device>, String> {
        Ok(self.ask::<LightsReply>(&Request::Lights)?.devices)
    }

    pub fn set_lights(&self, device: &str, colors: &[foxden_core::Color]) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::SetLights { device, colors })
            .map(drop)
    }

    /// The fan hubs the service lights; an older service refuses.
    pub fn hub_lights(&self, lengths: &BTreeMap<String, usize>) -> Result<HubLights, String> {
        self.ask(&Request::HubLights { lengths })
    }

    /// One frame for every hub, in as few requests as fit a line.
    pub fn set_hub_lights(&self, frames: &[HubFrame]) -> Result<HubSent, String> {
        let mut sent = HubSent {
            open: true,
            failed: BTreeMap::new(),
        };
        for batch in batches(frames, MAX_REQUEST) {
            let reply: HubSent = self.ask(&Request::SetHubLights { frames: batch })?;
            sent.open &= reply.open;
            sent.failed.extend(reply.failed);
        }
        Ok(sent)
    }

    /// Hands the hubs of this opening back to their firmware.
    pub fn release_hub_lights(&self, generation: u64) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::ReleaseHubLights { generation })
            .map(drop)
    }

    pub fn set_hub_hardware(
        &self,
        device: &str,
        setting: Option<&foxden_core::HardwareSetting>,
    ) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::SetHubHardware { device, setting })
            .map(drop)
    }

    /// Has the installed service install, remove or rename VB-CABLE; says what it did.
    /// What each SMBus answers, read by the service.
    pub fn smbus_report(&self) -> Result<Vec<String>, String> {
        #[derive(Deserialize)]
        struct Lines {
            lines: Vec<String>,
        }
        Ok(self.ask::<Lines>(&Request::SmbusReport)?.lines)
    }

    pub fn virtual_mic(&self, action: &str) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Done {
            text: String,
        }
        Ok(self.ask::<Done>(&Request::VirtualMic { action })?.text)
    }

    /// Has the service close what its own list names within `scope`.
    pub fn close_conflicts(&self, scope: Scope) -> Result<Closed, String> {
        self.ask(&Request::CloseConflicts(scope))
    }

    pub fn get_config(&self) -> Result<FanConfig, String> {
        Ok(self.ask::<ConfigReply>(&Request::GetConfig)?.config)
    }

    /// A config the service rejects comes back as the service's own error text.
    pub fn set_config(&self, config: &FanConfig) -> Result<(), String> {
        self.ask::<IgnoredAny>(&Request::SetConfig { config })
            .map(drop)
    }

    /// Asks a service run from a console to release the fans and exit. An installed one refuses.
    /// Sent on a connection of its own: an answer the service still owes must not keep the
    /// request to stop from reaching it.
    pub fn shutdown(&self) -> Result<(), String> {
        let line = serde_json::to_string(&Request::Shutdown).map_err(|e| e.to_string())?;
        let (reply, answer) = mpsc::channel();
        let pipe = self.pipe.clone();
        thread::spawn(move || reply.send(talk(&pipe, &mut None, &line)));
        let line = answer
            .recv_timeout(self.timeout)
            .map_err(|_| SILENT.to_string())??;
        verdict::<IgnoredAny>(&line).map(drop)
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
        verdict(&line)
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

/// Frames split so each batch's colours stay under `limit` bytes; a lone frame over it goes alone.
fn batches(frames: &[HubFrame], limit: usize) -> Vec<&[HubFrame]> {
    let mut out = Vec::new();
    let (mut start, mut size) = (0, 0);
    for (i, f) in frames.iter().enumerate() {
        let len = f.device.len() + f.colors.len() + 32;
        if i > start && size + len > limit {
            out.push(&frames[start..i]);
            (start, size) = (i, 0);
        }
        size += len;
    }
    if start < frames.len() {
        out.push(&frames[start..]);
    }
    out
}

/// What a reply line holds, or the service's reason for refusing.
fn verdict<T: DeserializeOwned>(line: &str) -> Result<T, String> {
    let odd = |e| format!("the service sent a reply that makes no sense: {e}");
    let verdict: Verdict = serde_json::from_str(line).map_err(odd)?;
    if !verdict.ok {
        return Err(verdict
            .error
            .unwrap_or_else(|| "the service refused the request".into()));
    }
    serde_json::from_str(line).map_err(odd)
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
            Ok(file) => {
                owner::verify(&file)?;
                return Ok(BufReader::new(file));
            }
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
