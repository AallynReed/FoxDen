use std::any::Any;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use foxrgb_core::conflicts::{Closed, Scope};
use foxrgb_core::{Backend, Color};
use foxrgb_fan::{Controller, FanBackend, FanConfig, Probe, Status};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

use crate::pipe;

pub const MAX_LINE: usize = 64 * 1024;
const LOG_CAP: u64 = 1024 * 1024;
const NO_SHUTDOWN: &str =
    "shutdown is for console mode; an installed service is stopped with `foxrgb-service stop`";
/// Waits before a control loop that panicked is started again. One more panic ends the process.
const BACK_OFF: [Duration; 3] = if cfg!(test) {
    [Duration::from_millis(10); 3]
} else {
    [
        Duration::from_secs(1),
        Duration::from_secs(5),
        Duration::from_secs(30),
    ]
};

/// How often the control loop makes a pass.
const PERIOD: Duration = Duration::from_secs(1);
/// A pass that takes longer than this is stuck, and the fans go back to the firmware.
const STALL: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(5)
};
/// How long listing the hardware may take before the service goes on without it.
const LISTING: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(10)
};
/// How long a stop waits for a pass of the control loop to end.
const PATIENCE: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(3)
};
const STUCK: &str = "the control loop is stuck";
/// The app's folder under `%APPDATA%`, named after its identifier.
const APP_FOLDER: &str = "com.foxrgb.desktop";

#[derive(Clone, Copy)]
pub enum Mode {
    Console,
    Service,
}

/// `FOXRGB_DATA_DIR` if set. Otherwise a console run uses the `service` folder beside the app's
/// settings, as a service the app starts does, and an installed service `%ProgramData%\FoxRGB`.
pub fn data_dir(mode: Mode) -> PathBuf {
    let under = |var: &str| PathBuf::from(std::env::var_os(var).unwrap_or_default());
    match (std::env::var_os("FOXRGB_DATA_DIR"), mode) {
        (Some(dir), _) if !dir.is_empty() => dir.into(),
        (_, Mode::Console) => under("APPDATA").join(APP_FOLDER).join("service"),
        (_, Mode::Service) => under("ProgramData").join("FoxRGB"),
    }
}

/// Opens one of the service's own files. A link put in its place is opened as the link itself,
/// and refused.
fn open(path: &Path, options: &mut OpenOptions) -> std::io::Result<File> {
    let file = options
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::other("it is a link"));
    }
    Ok(file)
}

fn timestamp() -> String {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Appends a line to service.log, keeping one older file once it grows past the cap.
pub fn log(dir: &Path, message: &str) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock();
    let line = format!("{} {message}\n", timestamp());
    // Not eprint!, which panics once stderr is a closed pipe.
    #[cfg(not(test))]
    let _ = std::io::stderr().write_all(line.as_bytes());
    let path = dir.join("service.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_CAP) {
        let _ = fs::rename(&path, dir.join("service.log.old"));
    }
    let _ = fs::create_dir_all(dir);
    if let Ok(mut file) = open(&path, OpenOptions::new().create(true).append(true)) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// A missing file reads as the default value.
fn load<T: DeserializeOwned + Default>(dir: &Path, file: &str) -> Result<T, String> {
    let path = dir.join(file);
    let read = open(&path, OpenOptions::new().read(true)).and_then(|mut file| {
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(text)
    });
    match read {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn save<T: Serialize>(dir: &Path, file: &str, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let path = dir.join(file);
    let tmp = dir.join(format!("{file}.tmp"));
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        // A leftover, or a link put there, goes first: the link itself, never what it names.
        let _ = fs::remove_file(&tmp);
        let mut file = open(
            &tmp,
            OpenOptions::new().write(true).create(true).truncate(true),
        )?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    };
    write().map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_config(dir: &Path) -> Result<FanConfig, String> {
    load(dir, "fans.json")
}

pub fn save_config(dir: &Path, config: &FanConfig) -> Result<(), String> {
    save(dir, "fans.json", config)
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Status,
    GetConfig,
    SetConfig {
        config: FanConfig,
    },
    Shutdown,
    /// Lighting on the SMBus, Super I/O and vendor WMI, which only a service running as SYSTEM can reach.
    Lights,
    SetLights {
        device: String,
        colors: Vec<Color>,
    },
    /// Closes software from the service's own list; the request only narrows it.
    CloseConflicts(Scope),
    /// Lighting of the fan hubs whose fans the service drives, so one process owns each hub.
    HubLights {
        #[serde(default)]
        lengths: BTreeMap<String, usize>,
    },
    SetHubLights {
        frames: Vec<HubFrame>,
    },
    /// A release for an earlier opening than the current one is ignored.
    ReleaseHubLights {
        #[serde(default)]
        generation: Option<u64>,
    },
    #[cfg(test)]
    Panic,
}

#[derive(Deserialize)]
struct HubFrame {
    device: String,
    /// `rrggbb` per LED.
    colors: String,
}

struct State {
    controller: Controller,
    status: Status,
}

/// What opening the hubs' lighting gives: the backends and their resizable zones.
pub type HubOpener = fn(&BTreeMap<String, usize>) -> (Vec<Box<dyn Backend>>, Vec<foxrgb_hid::Zone>);

struct Hub {
    backend: Box<dyn Backend>,
    ids: Vec<String>,
}

#[derive(Default)]
struct Hubs {
    open: Option<(Vec<Hub>, Vec<foxrgb_hid::Zone>)>,
    lengths: BTreeMap<String, usize>,
    generation: u64,
}

impl Hubs {
    fn ids(&self) -> Vec<String> {
        let hubs = self.open.iter().flat_map(|(hubs, _)| hubs);
        hubs.flat_map(|h| h.ids.clone()).collect()
    }
}

/// The lengths that belong to `ids`.
fn lengths_of(lengths: &BTreeMap<String, usize>, ids: &[String]) -> BTreeMap<String, usize> {
    let ours = |key: &String| {
        ids.iter().any(|id| {
            key.strip_prefix(id.as_str())
                .is_some_and(|r| r.starts_with('#'))
        })
    };
    lengths
        .iter()
        .filter(|(k, _)| ours(k))
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

/// The combined hubs' lighting, through the same handles as their fans.
pub fn open_hubs(
    lengths: &BTreeMap<String, usize>,
) -> (Vec<Box<dyn Backend>>, Vec<foxrgb_hid::Zone>) {
    let ids = foxrgb_xpack::combined();
    foxrgb_hid::port::share(&ids);
    foxrgb_hid::set_lengths(lengths.clone());
    let found = foxrgb_hid::probe_picked(&foxrgb_xpack::detectors(), foxrgb_hid::Pick::Only(&ids));
    (found, foxrgb_hid::resizable())
}

pub struct Host {
    pub dir: PathBuf,
    mode: Mode,
    /// The sensors. The control loop keeps them locked for a whole pass and reads them with
    /// `state` unlocked, so a read that hangs holds neither the fans nor the replies.
    probe: Mutex<Probe>,
    state: Mutex<State>,
    /// When the pass now under way began.
    busy: Mutex<Option<Instant>>,
    /// True while the config in force is not the one on disk. Held by every request that
    /// writes a file, which keeps those writes in order.
    unsaved: Mutex<bool>,
    stop: AtomicBool,
    /// Why the process should end: a shutdown request, or a control loop that keeps panicking.
    ended: Mutex<Option<Result<(), String>>>,
    /// SMBus, Super I/O and WMI lighting, opened on the first request for it.
    lights: Mutex<Option<Vec<Box<dyn Backend>>>>,
    hubs: Mutex<Hubs>,
    hub_opener: HubOpener,
    closer: fn(Scope) -> Closed,
}

impl Host {
    pub fn new(
        dir: PathBuf,
        hardware: impl FnOnce() -> Vec<Box<dyn FanBackend>> + Send + 'static,
        mode: Mode,
    ) -> Host {
        let mut controller = Controller::new(list(&dir, hardware), FanConfig::default());
        if let Err(e) = load_config(&dir).and_then(|config| controller.set_config(config)) {
            log(
                &dir,
                &format!("ignoring fans.json, running with an empty config: {e}"),
            );
        }
        let probe = controller.take_probe();
        let state = State {
            controller,
            status: Status::default(),
        };
        Host {
            dir,
            mode,
            probe: Mutex::new(probe),
            state: Mutex::new(state),
            busy: Mutex::new(None),
            unsaved: Mutex::new(false),
            stop: AtomicBool::new(false),
            ended: Mutex::new(None),
            lights: Mutex::new(None),
            hubs: Mutex::default(),
            hub_opener: open_hubs,
            closer: crate::close::run,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        locked(&self.state)
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Asks the process to end. The first reason given is the one that counts.
    pub fn end(&self, outcome: Result<(), String>) {
        locked(&self.ended).get_or_insert(outcome);
    }

    /// Answers one request line with one reply line. The flag is set when the process should
    /// end once the reply has been sent.
    pub fn handle(&self, line: &str) -> (String, bool) {
        if line.len() > MAX_LINE {
            let reply = fail(format!("line longer than {MAX_LINE} bytes"));
            return (reply.to_string(), false);
        }
        let mut shutdown = false;
        let reply = match serde_json::from_str(line) {
            Ok(Request::Status) => {
                let state = self.lock();
                json!({
                    "ok": true,
                    "status": state.status,
                    "version": env!("CARGO_PKG_VERSION"),
                })
            }
            Ok(Request::GetConfig) => {
                json!({"ok": true, "config": self.lock().controller.config()})
            }
            Ok(Request::SetConfig { config }) => done(self.set_config(config)),
            Ok(Request::Shutdown) => match self.mode {
                Mode::Console => {
                    shutdown = true;
                    json!({"ok": true})
                }
                Mode::Service => fail(NO_SHUTDOWN.into()),
            },
            Ok(Request::Lights) => {
                let mut lights = locked(&self.lights);
                if lights.as_ref().is_none_or(|found| found.is_empty()) {
                    let mut found = foxrgb_smbus::probe().unwrap_or_default();
                    found.extend(foxrgb_superio::lights());
                    found.extend(foxrgb_wmi::lights());
                    *lights = Some(found);
                }
                let devices: Vec<_> = lights.iter().flatten().flat_map(|b| b.devices()).collect();
                json!({"ok": true, "devices": devices})
            }
            Ok(Request::SetLights { device, colors }) => {
                let mut lights = locked(&self.lights);
                let backend = lights
                    .iter_mut()
                    .flatten()
                    .find(|b| b.devices().iter().any(|d| d.id == device));
                done(match backend {
                    Some(backend) => backend.push(&device, &colors),
                    None => Err(format!("unknown device {device}")),
                })
            }
            Ok(Request::CloseConflicts(scope)) => {
                let out = (self.closer)(scope);
                json!({"ok": true, "closed": out.closed, "left": out.left})
            }
            Ok(Request::HubLights { lengths }) => self.hub_lights(lengths),
            Ok(Request::SetHubLights { frames }) => {
                let open = locked(&self.hubs).open.is_some();
                json!({"ok": true, "open": open, "failed": self.set_hub_lights(frames)})
            }
            Ok(Request::ReleaseHubLights { generation }) => {
                let mut hubs = locked(&self.hubs);
                if generation.is_none_or(|g| g == hubs.generation) {
                    hubs.open = None;
                }
                json!({"ok": true})
            }
            #[cfg(test)]
            Ok(Request::Panic) => panic!("the request panics on purpose"),
            Err(e) => fail(format!("bad request: {e}")),
        };
        (reply.to_string(), shutdown)
    }

    /// Opens the hubs, again only when the lengths of their zones changed.
    fn hub_lights(&self, lengths: BTreeMap<String, usize>) -> Value {
        let mut hubs = locked(&self.hubs);
        let ids = hubs.ids();
        if hubs.open.is_none() || lengths_of(&lengths, &ids) != lengths_of(&hubs.lengths, &ids) {
            hubs.open = None;
            let (found, zones) = (self.hub_opener)(&lengths);
            let found = found
                .into_iter()
                .map(|backend| Hub {
                    ids: backend.devices().into_iter().map(|d| d.id).collect(),
                    backend,
                })
                .collect();
            hubs.open = Some((found, zones));
            hubs.generation += 1;
        }
        hubs.lengths = lengths;
        let generation = hubs.generation;
        let (found, zones) = hubs.open.as_ref().expect("opened above");
        let devices: Vec<_> = found.iter().flat_map(|h| h.backend.devices()).collect();
        let zones: Vec<_> = zones
            .iter()
            .filter(|z| found.iter().any(|h| h.ids.contains(&z.device)))
            .map(|z| {
                json!({"device": z.device, "index": z.index, "name": z.name,
                    "leds": z.leds, "min": z.min, "max": z.max})
            })
            .collect();
        json!({"ok": true, "devices": devices, "zones": zones, "generation": generation})
    }

    /// Pushes one frame to every hub, the hubs side by side so a slow one holds up only itself.
    fn set_hub_lights(&self, frames: Vec<HubFrame>) -> BTreeMap<String, String> {
        let mut failed = BTreeMap::new();
        let mut hubs = locked(&self.hubs);
        let Some((found, _)) = hubs.open.as_mut() else {
            let closed = "the hub lights are not open";
            return frames
                .into_iter()
                .map(|f| (f.device, closed.into()))
                .collect();
        };
        let mut work: Vec<Vec<(String, Vec<Color>)>> = found.iter().map(|_| Vec::new()).collect();
        for frame in frames {
            let hub = found.iter().position(|h| h.ids.contains(&frame.device));
            match (hub, foxrgb_core::from_hex(&frame.colors)) {
                (Some(i), Ok(colors)) => work[i].push((frame.device, colors)),
                (None, _) => {
                    failed.insert(frame.device, "unknown device".into());
                }
                (_, Err(e)) => {
                    failed.insert(frame.device, e);
                }
            }
        }
        let push = |backend: &mut Box<dyn Backend>, frames: Vec<(String, Vec<Color>)>| {
            let mut failed: Vec<(String, String)> = frames
                .into_iter()
                .filter_map(|(id, colors)| backend.push(&id, &colors).err().map(|e| (id, e)))
                .collect();
            if let Err(e) = backend.flush() {
                failed.push((String::new(), e));
            }
            failed
        };
        thread::scope(|scope| {
            let jobs: Vec<_> = found
                .iter_mut()
                .zip(work)
                .filter(|(_, frames)| !frames.is_empty())
                .map(|(hub, frames)| scope.spawn(|| push(&mut hub.backend, frames)))
                .collect();
            for job in jobs {
                match job.join() {
                    Ok(out) => failed.extend(out),
                    Err(panic) => {
                        failed.insert(String::new(), panic_text(&*panic).to_string());
                    }
                }
            }
        });
        failed
    }

    fn set_config(&self, config: FanConfig) -> Result<(), String> {
        // Orders the file writes. The control loop never takes this lock.
        let mut unsaved = locked(&self.unsaved);
        {
            let mut state = self.lock();
            if *state.controller.config() != config {
                state.controller.set_config(config.clone())?;
                *unsaved = true;
            }
        }
        if *unsaved {
            save_config(&self.dir, &config)
                .map_err(|e| format!("config applied but not saved: {e}"))?;
            *unsaved = false;
            log(&self.dir, "config changed and saved");
        }
        Ok(())
    }

    fn tick(&self, dt: f32) {
        // Kept for the whole pass, so the hardware is not swapped in the middle of it.
        let mut probe = locked(&self.probe);
        self.pass(&mut probe, dt);
    }

    /// Reads the sensors, then sets the fans.
    fn pass(&self, probe: &mut Probe, dt: f32) {
        *locked(&self.busy) = Some(Instant::now());
        let readings = probe.read(dt);
        {
            let mut state = self.lock();
            // A pass that was stuck can get here after the fans were handed back for good.
            if !self.stopping() {
                state.status = state.controller.apply(dt, readings);
            }
        }
        *locked(&self.busy) = None;
    }

    fn control_loop(&self) {
        let mut last = Instant::now();
        while !self.stopping() {
            let now = Instant::now();
            self.tick((now - last).as_secs_f32());
            last = now;
            while !self.stopping() && now.elapsed() < PERIOD {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    /// Watches the control loop. When a pass gets stuck, the fans go back to the firmware and
    /// every fan says why until the loop is back.
    fn watch(&self) {
        // Whether the fans were handed back, and whether the log knows they could not be.
        let (mut stuck, mut held) = (false, false);
        while !self.stopping() {
            thread::sleep(STALL / 20);
            let began = *locked(&self.busy);
            let late = began.is_some_and(|began| began.elapsed() > STALL);
            if late && !stuck {
                stuck = self.stalled();
                if !stuck && !held {
                    held = true;
                    let fans = "with the fans, which cannot be handed back from here";
                    log(&self.dir, &format!("{STUCK} {fans}"));
                }
            } else if !late && (stuck || held) {
                (stuck, held) = (false, false);
                log(&self.dir, "the control loop is running again");
            }
        }
    }

    /// Hands the fans back because a pass is stuck. False when that pass has the fans itself,
    /// which shows in the state staying locked.
    fn stalled(&self) -> bool {
        let Some(mut state) = within(&self.state, Duration::from_millis(200)) else {
            return false;
        };
        if self.stopping() {
            return true;
        }
        let note = match state.controller.release_all() {
            Ok(()) => {
                state.status.fans.iter_mut().for_each(|fan| fan.duty = None);
                format!("{STUCK}; the fans are back under firmware control")
            }
            Err(e) => format!("{STUCK} and some fans were not released: {e}"),
        };
        // What was read before the pass got stuck is no longer current.
        let status = &mut state.status;
        status
            .sensors
            .iter_mut()
            .for_each(|sensor| sensor.value = None);
        for fan in &mut status.fans {
            fan.failsafe = false;
            fan.error = Some(note.clone());
        }
        drop(state);
        log(
            &self.dir,
            &format!("no pass finished within {STALL:?}: {note}"),
        );
        true
    }

    /// Runs the control loop, starting it again after a panic until the back-off runs out.
    fn supervise(&self) {
        let mut waits = BACK_OFF.iter();
        loop {
            let Err(panic) = catch_unwind(AssertUnwindSafe(|| self.control_loop())) else {
                return;
            };
            let why = panic_text(&*panic);
            *locked(&self.busy) = None;
            // A second panic in here must not end this thread before the process is told.
            let _ = catch_unwind(AssertUnwindSafe(|| self.release_all()));
            let Some(&wait) = waits.next() else {
                log(
                    &self.dir,
                    &format!("control loop panicked again ({why}), giving up"),
                );
                self.end(Err(format!("the control loop keeps panicking: {why}")));
                return;
            };
            log(
                &self.dir,
                &format!("control loop panicked ({why}), starting it again in {wait:?}"),
            );
            let since = Instant::now();
            while since.elapsed() < wait {
                if self.stopping() {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub fn release_all(&self) {
        // Only a pass that is stuck with the fans themselves keeps the state this long.
        let Some(mut state) = within(&self.state, PATIENCE) else {
            log(
                &self.dir,
                &format!("{STUCK} with the fans, which were not released"),
            );
            return;
        };
        match state.controller.release_all() {
            Ok(()) => {
                let fans = &mut state.status.fans;
                let driven = fans.iter().filter(|fan| fan.duty.is_some()).count();
                fans.iter_mut().for_each(|fan| fan.duty = None);
                log(
                    &self.dir,
                    &format!("released fans to firmware control ({driven} had a duty set)"),
                );
            }
            Err(e) => log(&self.dir, &format!("some fans were not released: {e}")),
        }
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The lock, unless it stays taken for longer than `patience`.
fn within<T>(mutex: &Mutex<T>, patience: Duration) -> Option<MutexGuard<'_, T>> {
    let deadline = Instant::now() + patience;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() >= deadline => return None,
            Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Lists the hardware on a thread of its own, so that a driver which hangs or panics costs
/// its sensors and not the service.
fn list(
    dir: &Path,
    hardware: impl FnOnce() -> Vec<Box<dyn FanBackend>> + Send + 'static,
) -> Vec<Box<dyn FanBackend>> {
    let listing = thread::spawn(hardware);
    let deadline = Instant::now() + LISTING;
    while !listing.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    if !listing.is_finished() {
        let late = format!("listing the hardware took longer than {LISTING:?}");
        log(dir, &format!("{late}, going on without it"));
        Vec::new()
    } else {
        listing.join().unwrap_or_else(|panic| {
            let why = panic_text(&*panic);
            log(
                dir,
                &format!("listing the hardware failed ({why}), going on without it"),
            );
            Vec::new()
        })
    }
}

fn panic_text(panic: &(dyn Any + Send)) -> &str {
    if let Some(text) = panic.downcast_ref::<&str>() {
        text
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text
    } else {
        "no message"
    }
}

fn done(result: Result<(), String>) -> Value {
    match result {
        Ok(()) => json!({"ok": true}),
        Err(e) => fail(e),
    }
}

fn fail(error: String) -> Value {
    json!({"ok": false, "error": error})
}

pub struct Running {
    host: Arc<Host>,
    pipe_name: String,
    control: Option<JoinHandle<()>>,
    watch: Option<JoinHandle<()>>,
    pipe: JoinHandle<()>,
}

/// Starts the control loop and the pipe server.
pub fn start(
    dir: PathBuf,
    hardware: impl FnOnce() -> Vec<Box<dyn FanBackend>> + Send + 'static,
    mode: Mode,
    pipe_name: &str,
) -> Result<Running, String> {
    // Taking the pipe name first keeps a second instance away from the fans.
    let first = pipe::create(pipe_name, true)
        .map_err(|e| format!("{e} (is foxrgb-service already running?)"))?;
    let host = Arc::new(Host::new(dir, hardware, mode));
    let version = env!("CARGO_PKG_VERSION");
    log(
        &host.dir,
        &format!("started, version {version}, listening on {pipe_name}"),
    );

    let h = host.clone();
    let control = thread::spawn(move || h.supervise());
    let h = host.clone();
    let watch = thread::spawn(move || h.watch());
    let (h, name) = (host.clone(), pipe_name.to_string());
    let pipe = thread::spawn(move || pipe::serve(&h, &name, first));
    Ok(Running {
        host,
        pipe_name: pipe_name.to_string(),
        control: Some(control),
        watch: Some(watch),
        pipe,
    })
}

impl Running {
    /// Set once the process should end: `Ok` after a shutdown request, `Err` when the control
    /// loop has given up.
    pub fn ended(&self) -> Option<Result<(), String>> {
        locked(&self.host.ended).clone()
    }

    /// Stops both threads and hands the fans back. Dropping does the same.
    pub fn stop(self) {}
}

impl Drop for Running {
    fn drop(&mut self) {
        self.host.stop.store(true, Ordering::Relaxed);
        // Fans first: the pipe shutdown below must not be able to delay the release.
        if let Some(control) = self.control.take() {
            let deadline = Instant::now() + PATIENCE;
            while !control.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            // A loop that is stuck is left behind; it sets nothing once it comes back.
            if control.is_finished() {
                let _ = control.join();
            } else {
                log(&self.host.dir, &format!("{STUCK}, stopping without it"));
            }
        }
        self.host.release_all();
        // Lighting after the fans, so a hub's last driver is the one that hands it back.
        locked(&self.host.hubs).open = None;
        if let Some(watch) = self.watch.take() {
            let _ = watch.join();
        }
        while !self.pipe.is_finished() {
            pipe::wake(&self.pipe_name);
            thread::sleep(Duration::from_millis(20));
        }
        log(&self.host.dir, "stopped");
    }
}

/// Every sensor and fan this machine offers: read-only sources, then the board and CPU through PawnIO.
pub fn hardware() -> Vec<Box<dyn FanBackend>> {
    foxrgb_hid::port::share(&foxrgb_xpack::combined());
    let usb = thread::spawn(foxrgb_xpack::fan_backends);
    let mut backends = foxrgb_hwmon::backends();
    let amd_card = backends
        .iter_mut()
        .any(|b| b.sensors().iter().any(|s| s.id().starts_with("gpu")));
    backends.extend(foxrgb_superio::backends());
    // A USB hub that never answers costs only the USB fans, not the whole listing.
    let deadline = Instant::now() + LISTING / 2;
    while !usb.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    if usb.is_finished()
        && let Ok(more) = usb.join()
    {
        backends.extend(more);
    }
    if amd_card {
        backends.push(Box::new(RelayBackend));
    }
    backends
}

/// The graphics card fan. AMD's ADLX crashes inside a service, so the service only works out
/// the duty and the app, in the user's session, applies it.
struct RelayBackend;

struct RelayFan;

impl foxrgb_fan::Fan for RelayFan {
    fn id(&self) -> &str {
        "gpu0/fan"
    }
    fn name(&self) -> &str {
        "Graphics card fan"
    }
    fn rpm(&mut self) -> Option<u32> {
        None
    }
    fn set_duty(&mut self, _duty: f32) -> Result<(), String> {
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        Ok(())
    }
}

impl FanBackend for RelayBackend {
    fn name(&self) -> &str {
        "relay"
    }
    fn sensors(&mut self) -> Vec<Box<dyn foxrgb_fan::Sensor>> {
        Vec::new()
    }
    fn fans(&mut self) -> Vec<Box<dyn foxrgb_fan::Fan>> {
        vec![Box::new(RelayFan)]
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::LazyLock;
    use std::sync::atomic::AtomicU32;

    use foxrgb_fan::{Curve, Fan, FanRule, Sensor};

    use super::*;

    /// A backend with one fan that records its release.
    #[derive(Clone, Default)]
    struct Probe {
        released: Arc<AtomicBool>,
        /// How many more writes panic.
        panics: Arc<AtomicU32>,
        /// Refuses to be released.
        stuck: bool,
        /// Writes do not return while this is set.
        hangs: Arc<AtomicBool>,
    }

    impl FanBackend for Probe {
        fn name(&self) -> &str {
            "probe"
        }
        fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
            Vec::new()
        }
        fn fans(&mut self) -> Vec<Box<dyn Fan>> {
            vec![Box::new(self.clone())]
        }
    }

    impl Fan for Probe {
        fn id(&self) -> &str {
            "probe"
        }
        fn name(&self) -> &str {
            "probe"
        }
        fn rpm(&mut self) -> Option<u32> {
            None
        }
        fn set_duty(&mut self, _duty: f32) -> Result<(), String> {
            while self.hangs.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            let left = self.panics.load(Ordering::Relaxed);
            if left > 0 {
                self.panics.store(left - 1, Ordering::Relaxed);
                panic!("the probe fan panics on purpose");
            }
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> {
            if self.stuck {
                return Err("stuck".into());
            }
            self.released.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    /// A sensor named as the sample curve wants it, which stops answering while `held` is set.
    #[derive(Clone, Default)]
    struct Slow {
        held: Arc<AtomicBool>,
    }

    impl FanBackend for Slow {
        fn name(&self) -> &str {
            "slow"
        }
        fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
            vec![Box::new(self.clone())]
        }
        fn fans(&mut self) -> Vec<Box<dyn Fan>> {
            Vec::new()
        }
    }

    impl Sensor for Slow {
        fn id(&self) -> &str {
            "cpu"
        }
        fn name(&self) -> &str {
            "cpu"
        }
        fn read(&mut self) -> Option<f32> {
            while self.held.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            Some(40.0)
        }
    }

    /// A running host whose probe fan follows the slow sensor.
    fn start_slow(name: &str, probe: &Probe, slow: &Slow) -> Running {
        let hardware = {
            let (probe, slow) = (probe.clone(), slow.clone());
            move || {
                vec![
                    Box::new(slow.clone()) as Box<dyn FanBackend>,
                    Box::new(probe.clone()),
                ]
            }
        };
        let running = start(probe_dir(name), hardware, Mode::Console, &pipe_name(name));
        let running = running.unwrap();
        wait_for("the first duty", || driven(&running.host));
        running
    }

    /// A data directory whose config drives the probe fan.
    fn probe_dir(name: &str) -> PathBuf {
        let dir = scratch(name);
        let mut config = sample();
        config.rules.push(FanRule {
            fan: "probe".into(),
            curve: "cpu".into(),
            min_duty: 0.0,
            max_duty: 1.0,
            ramp: 1.0,
        });
        save_config(&dir, &config).unwrap();
        dir
    }

    fn hardware(probe: &Probe) -> impl Fn() -> Vec<Box<dyn FanBackend>> + Send + Sync + 'static {
        let probe = probe.clone();
        move || vec![Box::new(probe.clone())]
    }

    fn start_probe(name: &str, probe: &Probe) -> Running {
        let (dir, pipe) = (probe_dir(name), pipe_name(name));
        start(dir, hardware(probe), Mode::Console, &pipe).unwrap()
    }

    fn driven(host: &Host) -> bool {
        let state = host.lock();
        state.status.fans.iter().any(|fan| fan.duty.is_some())
    }

    pub fn pipe_name(name: &str) -> String {
        format!(
            r"\\.\pipe\foxrgb-service-test-{name}-{}",
            std::process::id()
        )
    }

    pub fn wait_for(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn log_of(dir: &Path) -> String {
        fs::read_to_string(dir.join("service.log")).unwrap()
    }

    #[test]
    fn stop_releases_a_driven_fan() {
        let probe = Probe::default();
        let running = start_probe("release", &probe);
        wait_for("the first duty", || driven(&running.host));
        assert!(!probe.released.load(Ordering::Relaxed));
        running.stop();
        assert!(probe.released.load(Ordering::Relaxed));
    }

    #[test]
    fn dropping_the_host_releases_a_driven_fan() {
        let probe = Probe::default();
        let running = start_probe("drop", &probe);
        wait_for("the first duty", || driven(&running.host));
        drop(running);
        assert!(probe.released.load(Ordering::Relaxed));
    }

    #[test]
    fn a_control_loop_that_panicked_is_started_again() {
        let probe = Probe::default();
        probe.panics.store(2, Ordering::Relaxed);
        let running = start_probe("restart", &probe);
        wait_for("the first duty", || driven(&running.host));
        assert!(probe.released.load(Ordering::Relaxed));
        assert_eq!(running.ended(), None);
        let dir = running.host.dir.clone();
        running.stop();
        let log = log_of(&dir);
        assert_eq!(log.matches("the probe fan panics on purpose").count(), 2);
        assert!(!log.contains("giving up"), "{log}");
    }

    #[test]
    fn a_control_loop_that_keeps_panicking_ends_the_process() {
        let probe = Probe::default();
        probe.panics.store(u32::MAX, Ordering::Relaxed);
        let running = start_probe("panic", &probe);
        wait_for("the host to give up", || running.ended().is_some());
        assert!(running.ended().unwrap().is_err());
        assert!(probe.released.load(Ordering::Relaxed));
        // The pipe side still answers until the process stops it.
        assert_eq!(reply(&running.host, r#"{"cmd":"status"}"#)["ok"], true);
        let dir = running.host.dir.clone();
        running.stop();
        let log = log_of(&dir);
        assert_eq!(log.matches("starting it again").count(), BACK_OFF.len());
        assert!(log.contains("giving up"), "{log}");
    }

    #[test]
    fn a_read_that_hangs_sends_the_fans_back_and_holds_nothing_else() {
        let (probe, slow) = (Probe::default(), Slow::default());
        let running = start_slow("hang", &probe, &slow);
        assert!(!probe.released.load(Ordering::Relaxed));

        slow.held.store(true, Ordering::Relaxed);
        wait_for("the fans to be handed back", || {
            probe.released.load(Ordering::Relaxed)
        });
        // Requests are still answered, and say what is going on.
        let status = reply(&running.host, r#"{"cmd":"status"}"#);
        let fan = &status["status"]["fans"][0];
        assert_eq!(fan["duty"], Value::Null);
        assert!(fan["error"].as_str().unwrap().starts_with(STUCK), "{fan}");
        assert_eq!(status["status"]["sensors"][0]["value"], Value::Null);

        // Nor does a stop wait for the read.
        let asked = Instant::now();
        let dir = running.host.dir.clone();
        running.stop();
        assert!(asked.elapsed() < PATIENCE * 4, "{:?}", asked.elapsed());
        slow.held.store(false, Ordering::Relaxed);
        let log = log_of(&dir);
        assert!(log.contains("no pass finished"), "{log}");
        assert!(log.contains("stopping without it"), "{log}");
        assert!(log.ends_with("stopped\n"), "{log}");
    }

    #[test]
    fn a_pass_that_was_stuck_drives_the_fans_again_once_it_is_back() {
        let (probe, slow) = (Probe::default(), Slow::default());
        let running = start_slow("back", &probe, &slow);
        slow.held.store(true, Ordering::Relaxed);
        wait_for("the fans to be handed back", || {
            probe.released.load(Ordering::Relaxed)
        });
        assert!(!driven(&running.host));

        slow.held.store(false, Ordering::Relaxed);
        wait_for("the fans to be driven again", || driven(&running.host));
        let status = reply(&running.host, r#"{"cmd":"status"}"#);
        assert_eq!(status["status"]["fans"][0]["error"], Value::Null);
        assert_eq!(status["status"]["sensors"][0]["value"], 40.0);
        let dir = running.host.dir.clone();
        wait_for("the log to say so", || {
            log_of(&dir).contains("the control loop is running again")
        });
    }

    #[test]
    fn a_stuck_pass_sets_nothing_after_the_stop() {
        let (probe, slow) = (Probe::default(), Slow::default());
        let running = start_slow("late-pass", &probe, &slow);
        slow.held.store(true, Ordering::Relaxed);
        wait_for("the fans to be handed back", || {
            probe.released.load(Ordering::Relaxed)
        });
        let host = running.host.clone();
        running.stop();
        // The read comes back only now, and its pass must leave the fans alone.
        slow.held.store(false, Ordering::Relaxed);
        wait_for("the pass to end", || locked(&host.busy).is_none());
        assert!(!driven(&host));
    }

    #[test]
    fn a_write_that_hangs_is_logged_and_does_not_hold_up_the_stop() {
        let probe = Probe::default();
        let running = start_probe("hang-write", &probe);
        wait_for("the first duty", || driven(&running.host));
        let dir = running.host.dir.clone();

        probe.hangs.store(true, Ordering::Relaxed);
        wait_for("the log to say so", || {
            log_of(&dir).contains("with the fans, which cannot be handed back")
        });
        // Nothing can release a fan that is in the middle of a write, but the process can end.
        let asked = Instant::now();
        running.stop();
        assert!(asked.elapsed() < PATIENCE * 4, "{:?}", asked.elapsed());
        assert!(!probe.released.load(Ordering::Relaxed));
        probe.hangs.store(false, Ordering::Relaxed);
        let log = log_of(&dir);
        assert!(log.contains("which were not released"), "{log}");
        assert!(
            log.ends_with(
                "stopped
"
            ),
            "{log}"
        );
    }

    #[test]
    fn hardware_that_takes_too_long_to_list_is_left_out() {
        let dir = scratch("slow-list");
        let slow = || {
            thread::sleep(LISTING * 3);
            vec![Box::new(Probe::default()) as Box<dyn FanBackend>]
        };
        let asked = Instant::now();
        let host = Host::new(dir.clone(), slow, Mode::Console);
        assert!(asked.elapsed() < LISTING * 2, "{:?}", asked.elapsed());
        host.tick(1.0);
        assert!(fan_ids(&host).is_empty());
        assert!(log_of(&dir).contains("took longer"));
    }

    #[test]
    fn hardware_that_panics_while_it_is_listed_is_left_out() {
        let dir = scratch("panic-list");
        let broken = || -> Vec<Box<dyn FanBackend>> { panic!("this hardware cannot be listed") };
        let host = Host::new(dir.clone(), broken, Mode::Console);
        host.tick(1.0);
        assert!(fan_ids(&host).is_empty());
        assert!(log_of(&dir).contains("this hardware cannot be listed"));
    }

    pub fn scratch(name: &str) -> PathBuf {
        // Beside the test binary by default, so nothing is left in the system temp folder.
        let beside = || {
            std::env::current_exe()
                .unwrap()
                .with_file_name("foxrgb-service-tests")
        };
        let base = std::env::var_os("FOXRGB_DATA_DIR").map_or_else(beside, PathBuf::from);
        let dir = base.join(format!("test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn sample() -> FanConfig {
        FanConfig {
            curves: vec![Curve {
                name: "cpu".into(),
                sensors: vec!["cpu".into()],
                points: vec![(30.0, 0.25), (80.0, 1.0)],
                hysteresis: 2.0,
            }],
            rules: Vec::new(),
        }
    }

    /// A console host with no hardware.
    fn host(name: &str) -> Host {
        Host::new(scratch(name), Vec::new, Mode::Console)
    }

    fn reply(host: &Host, line: &str) -> Value {
        serde_json::from_str(&host.handle(line).0).unwrap()
    }

    fn fan_ids(host: &Host) -> Vec<String> {
        let status = reply(host, r#"{"cmd":"status"}"#);
        let fans = status["status"]["fans"].as_array().unwrap();
        let id = |fan: &Value| fan["id"].as_str().unwrap().to_string();
        fans.iter().map(id).collect()
    }

    #[test]
    fn close_conflicts_uses_only_the_scope() {
        static SEEN: Mutex<Vec<Scope>> = Mutex::new(Vec::new());
        fn fake(scope: Scope) -> Closed {
            SEEN.lock().unwrap().push(scope);
            Closed {
                closed: vec!["Fan Control".into()],
                left: Vec::new(),
            }
        }
        let mut host = host("close");
        host.closer = fake;
        let r = reply(
            &host,
            r#"{"cmd":"close_conflicts","lighting":true,"names":["explorer.exe"]}"#,
        );
        assert_eq!(
            r,
            json!({"ok": true, "closed": ["Fan Control"], "left": []})
        );
        assert_eq!(reply(&host, r#"{"cmd":"close_conflicts"}"#)["ok"], true);
        let lighting = Scope {
            lighting: true,
            ..Scope::default()
        };
        assert_eq!(*SEEN.lock().unwrap(), [lighting, Scope::default()]);
    }

    #[test]
    fn status_reports_the_version() {
        let r = reply(&host("status"), r#"{"cmd":"status"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["version"], env!("CARGO_PKG_VERSION"));
        assert!(r["status"]["fans"].is_array());
        assert!(r["status"]["sensors"].is_array());
        assert_eq!(r.get("sim"), None);
    }

    #[test]
    fn shutdown_is_for_console_mode_only() {
        let console = host("shutdown");
        let (line, shutdown) = console.handle(r#"{"cmd":"shutdown"}"#);
        assert_eq!(line, r#"{"ok":true}"#);
        assert!(shutdown);

        let service = Host::new(scratch("shutdown-service"), Vec::new, Mode::Service);
        let (line, shutdown) = service.handle(r#"{"cmd":"shutdown"}"#);
        let r: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(r["ok"], false);
        assert!(r["error"].is_string());
        assert!(!shutdown);
    }

    #[test]
    fn set_config_applies_and_saves() {
        let host = host("set");
        let line = json!({"cmd": "set_config", "config": sample()}).to_string();
        assert_eq!(reply(&host, &line), json!({"ok": true}));
        assert_eq!(
            reply(&host, r#"{"cmd":"get_config"}"#)["config"],
            json!(sample())
        );
        assert_eq!(load_config(&host.dir), Ok(sample()));
        assert!(!host.dir.join("fans.json.tmp").exists());
    }

    #[test]
    fn rejected_config_is_neither_applied_nor_saved() {
        let host = host("reject");
        let mut config = sample();
        config.curves[0].points[0].1 = 7.0;
        let r = reply(
            &host,
            &json!({"cmd": "set_config", "config": config}).to_string(),
        );
        assert_eq!(r["ok"], false);
        assert!(r["error"].is_string());
        assert_eq!(*host.lock().controller.config(), FanConfig::default());
        assert!(!host.dir.join("fans.json").exists());
    }

    #[test]
    fn a_full_log_is_rotated_once() {
        let dir = scratch("rotate");
        let full = vec![b'x'; LOG_CAP as usize + 1];
        fs::write(dir.join("service.log"), &full).unwrap();
        log(&dir, "first");
        log(&dir, "second");
        assert_eq!(fs::read(dir.join("service.log.old")).unwrap(), full);
        let fresh = fs::read_to_string(dir.join("service.log")).unwrap();
        assert_eq!(fresh.lines().count(), 2, "{fresh}");
        assert!(fresh.ends_with("second\n"), "{fresh}");
    }

    #[test]
    fn a_link_in_place_of_a_file_is_not_followed() {
        use std::os::windows::fs::symlink_file;
        let dir = scratch("links");
        let elsewhere = dir.join("elsewhere.txt");
        fs::write(&elsewhere, "untouched").unwrap();
        // Making a link takes a right that few accounts have. Without it there is nothing to try.
        if symlink_file(&elsewhere, dir.join("service.log")).is_err() {
            return;
        }
        log(&dir, "a line that must not land elsewhere");
        symlink_file(&elsewhere, dir.join("fans.json")).unwrap();
        assert!(load_config(&dir).is_err());
        fs::remove_file(dir.join("fans.json")).unwrap();
        symlink_file(&elsewhere, dir.join("fans.json.tmp")).unwrap();
        save_config(&dir, &sample()).unwrap();
        assert_eq!(load_config(&dir), Ok(sample()));
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "untouched");
    }

    #[test]
    fn a_console_run_and_an_installed_service_keep_their_files_apart() {
        // Only meaningful where the variable does not send both to one folder, as here.
        if std::env::var_os("FOXRGB_DATA_DIR").is_some_and(|dir| !dir.is_empty()) {
            assert_eq!(data_dir(Mode::Console), data_dir(Mode::Service));
            return;
        }
        let (console, service) = (data_dir(Mode::Console), data_dir(Mode::Service));
        assert!(
            console.ends_with(r"com.foxrgb.desktop\service"),
            "{console:?}"
        );
        assert!(service.ends_with("FoxRGB"), "{service:?}");
        assert_ne!(console, service);
    }

    #[test]
    fn a_failed_save_is_retried_by_the_same_request() {
        let host = host("retry");
        let blocker = host.dir.join("fans.json");
        fs::create_dir(&blocker).unwrap();
        let line = json!({"cmd": "set_config", "config": sample()}).to_string();
        assert_eq!(reply(&host, &line)["ok"], false);
        fs::remove_dir(&blocker).unwrap();
        assert_eq!(reply(&host, &line), json!({"ok": true}));
        assert_eq!(load_config(&host.dir), Ok(sample()));
    }

    #[test]
    fn bad_requests_get_an_error() {
        let host = host("bad");
        let lines = [
            "",
            "not json",
            "5",
            "{}",
            r#"{"cmd":"reboot"}"#,
            r#"{"cmd":"set_config"}"#,
            r#"{"cmd":"set_config","config":{"curves":5}}"#,
            r#"{"cmd":"set_sim","on":true}"#,
        ];
        for line in lines {
            let (text, shutdown) = host.handle(line);
            let r: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(r["ok"], false, "{line}");
            assert!(r["error"].is_string(), "{line}");
            assert!(!shutdown, "{line}");
        }
        assert_eq!(
            reply(&host, r#"{"cmd":"get_config"}"#)["config"],
            json!(FanConfig::default())
        );
        assert!(!host.dir.join("fans.json").exists());
    }

    #[test]
    fn oversized_line_is_rejected() {
        let line = format!(r#"{{"cmd":"status","pad":"{}"}}"#, "x".repeat(MAX_LINE));
        assert_eq!(reply(&host("big"), &line)["ok"], false);
    }

    #[test]
    fn missing_file_is_an_empty_config() {
        assert_eq!(load_config(&scratch("missing")), Ok(FanConfig::default()));
    }

    #[test]
    fn saved_config_is_loaded_at_start() {
        let dir = scratch("load");
        save_config(&dir, &sample()).unwrap();
        let host = Host::new(dir, Vec::new, Mode::Console);
        assert_eq!(*host.lock().controller.config(), sample());
    }

    #[test]
    fn invalid_file_means_empty_config_and_a_log_line() {
        let dir = scratch("invalid");
        fs::write(dir.join("fans.json"), "{ nope").unwrap();
        let host = Host::new(dir.clone(), Vec::new, Mode::Console);
        assert_eq!(*host.lock().controller.config(), FanConfig::default());
        assert!(log_of(&dir).contains("fans.json"));
    }

    type Pushes = Arc<Mutex<Vec<(String, Vec<Color>)>>>;
    type Writes = Arc<Mutex<Vec<(u8, bool)>>>;

    /// Hub lighting that records its pushes, and refuses an empty frame.
    struct Lit {
        id: &'static str,
        pushes: Pushes,
    }

    impl Backend for Lit {
        fn name(&self) -> &str {
            "lit"
        }
        fn devices(&self) -> Vec<foxrgb_core::Device> {
            let kind = foxrgb_core::DeviceKind::Other;
            vec![foxrgb_core::Device::grid(
                self.id, "Hub", kind, [0.0; 4], 2, 1,
            )]
        }
        fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
            if colors.is_empty() {
                return Err("refused".into());
            }
            locked(&self.pushes).push((device_id.into(), colors.to_vec()));
            Ok(())
        }
    }

    fn zone(device: &str, leds: usize) -> foxrgb_hid::Zone {
        foxrgb_hid::Zone {
            device: device.into(),
            index: 1,
            name: "Fan 1".into(),
            leds,
            min: 0,
            max: 34,
        }
    }

    #[test]
    fn hub_lights_get_the_zone_lengths_and_take_one_batch_per_frame() {
        static OPENED: Mutex<Vec<BTreeMap<String, usize>>> = Mutex::new(Vec::new());
        static PUSHES: LazyLock<Pushes> = LazyLock::new(Arc::default);
        fn opener(
            lengths: &BTreeMap<String, usize>,
        ) -> (Vec<Box<dyn Backend>>, Vec<foxrgb_hid::Zone>) {
            locked(&OPENED).push(lengths.clone());
            let lit = |id| {
                Box::new(Lit {
                    id,
                    pushes: PUSHES.clone(),
                }) as Box<dyn Backend>
            };
            let leds = lengths.get("hid:a#1").copied().unwrap_or(34);
            (
                vec![lit("hid:a"), lit("hid:b")],
                vec![zone("hid:a", leds), zone("hid:gone", 1)],
            )
        }
        let mut host = host("hubs");
        host.hub_opener = opener;
        let set = |frames: Value| {
            reply(
                &host,
                &json!({"cmd": "set_hub_lights", "frames": frames}).to_string(),
            )
        };
        assert_eq!(
            set(json!([{"device": "hid:a", "colors": "ff0000"}]))["failed"]["hid:a"],
            "the hub lights are not open"
        );

        let ask = |lengths: Value| {
            reply(
                &host,
                &json!({"cmd": "hub_lights", "lengths": lengths}).to_string(),
            )
        };
        let r = ask(json!({"hid:a#1": 12, "other#0": 3}));
        assert_eq!(r["ok"], true);
        assert_eq!(r["devices"].as_array().unwrap().len(), 2);
        assert_eq!(
            r["zones"],
            json!([{"device": "hid:a", "index": 1, "name": "Fan 1", "leds": 12, "min": 0, "max": 34}])
        );
        // Only a change to the hubs' own zones opens them again.
        ask(json!({"hid:a#1": 12, "other#0": 9}));
        assert_eq!(locked(&OPENED).len(), 1);
        assert_eq!(locked(&OPENED)[0]["hid:a#1"], 12);
        ask(json!({"hid:a#1": 20}));
        assert_eq!(locked(&OPENED).len(), 2);
        // An older app asks without lengths.
        assert_eq!(reply(&host, r#"{"cmd":"hub_lights"}"#)["ok"], true);
        assert_eq!(locked(&OPENED).len(), 3);

        let r = set(json!([
            {"device": "hid:a", "colors": "ff0000"},
            {"device": "hid:b", "colors": "00ff0000ff00"},
            {"device": "hid:b", "colors": ""},
            {"device": "hid:c", "colors": "000000"},
            {"device": "hid:a", "colors": "f"},
        ]));
        assert_eq!(r["ok"], true);
        assert_eq!(r["failed"]["hid:b"], "refused");
        assert_eq!(r["failed"]["hid:c"], "unknown device");
        assert!(
            r["failed"]["hid:a"]
                .as_str()
                .unwrap()
                .starts_with("bad colour")
        );
        let mut pushes = locked(&PUSHES).clone();
        pushes.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            pushes,
            [
                ("hid:a".into(), vec![Color(255, 0, 0)]),
                ("hid:b".into(), vec![Color(0, 255, 0), Color(0, 255, 0)]),
            ]
        );

        assert_eq!(r["open"], true);

        // A proxy dropped late must not close the hubs a newer one opened.
        let release = |g: u64| json!({"cmd": "release_hub_lights", "generation": g}).to_string();
        assert_eq!(reply(&host, &release(2)), json!({"ok": true}));
        assert!(locked(&host.hubs).open.is_some());
        assert_eq!(reply(&host, &release(3)), json!({"ok": true}));
        assert!(locked(&host.hubs).open.is_none());
        let r = set(json!([{"device": "hid:a", "colors": "ff0000"}]));
        assert_eq!(r["open"], false);
        let r = ask(json!({"hid:a#1": 20}));
        assert_eq!(
            (r["generation"].as_u64(), locked(&OPENED).len()),
            (Some(4), 4)
        );
        assert_eq!(reply(&host, r#"{"cmd":"release_hub_lights"}"#)["ok"], true);
        assert!(locked(&host.hubs).open.is_none());
    }

    /// One HID handle that notes, for each write, whether its device's lock was held.
    struct Wire {
        device: Arc<foxrgb_hid::port::Device>,
        writes: Writes,
    }

    impl foxrgb_hid::port::Raw for Wire {
        fn write(&self, data: &[u8]) -> foxrgb_hid::port::HidResult<usize> {
            locked(&self.writes).push((data[0], self.device.held_here()));
            thread::sleep(Duration::from_micros(200));
            Ok(data.len())
        }
        fn read_timeout(&self, _: &mut [u8], _: i32) -> foxrgb_hid::port::HidResult<usize> {
            Ok(0)
        }
        fn send_feature_report(&self, _: &[u8]) -> foxrgb_hid::port::HidResult<()> {
            Ok(())
        }
        fn get_feature_report(&self, _: &mut [u8]) -> foxrgb_hid::port::HidResult<usize> {
            Ok(0)
        }
        fn get_input_report(&self, _: &mut [u8]) -> foxrgb_hid::port::HidResult<usize> {
            Ok(0)
        }
    }

    /// Lighting that sends each frame as two reports, 1 then 2.
    struct TwoPart(foxrgb_hid::port::Port);

    impl Backend for TwoPart {
        fn name(&self) -> &str {
            "two part"
        }
        fn devices(&self) -> Vec<foxrgb_core::Device> {
            let kind = foxrgb_core::DeviceKind::Other;
            vec![foxrgb_core::Device::grid(
                "hid:combined",
                "Hub",
                kind,
                [0.0; 4],
                1,
                1,
            )]
        }
        fn push(&mut self, _: &str, _: &[Color]) -> Result<(), String> {
            self.0.write(&[1]).map_err(|e| e.to_string())?;
            self.0.write(&[2]).map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    /// Fans that send each duty as two reports, 3 then 4.
    struct Cooled(foxrgb_hid::port::Port);

    impl foxrgb_xpack::fan::FanHub for Cooled {
        fn id(&self) -> String {
            "fan:combined".into()
        }
        fn name(&self) -> String {
            "Hub".into()
        }
        fn channels(&self) -> Vec<foxrgb_xpack::fan::Channel> {
            vec![foxrgb_xpack::fan::Channel::fan("Fan 1")]
        }
        fn rpm(&mut self, _: usize) -> Option<u32> {
            None
        }
        fn set_duty(&mut self, _: usize, _: f32) -> Result<(), String> {
            self.0.write(&[3]).map_err(|e| e.to_string())?;
            self.0.write(&[4]).map_err(|e| e.to_string())?;
            Ok(())
        }
        fn release(&mut self, _: usize) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_combined_hub_is_opened_once_and_both_drivers_write_under_its_lock() {
        use foxrgb_hid::port::{self, Port, Raw};
        use foxrgb_hid::registry::{Bus, Info};
        static INFO: LazyLock<Info> = LazyLock::new(|| Info {
            vid: 0xF0F1,
            pid: 0x0001,
            interface: 0,
            usage_page: 0xFF42,
            usage: 1,
            path: "combined".into(),
            serial: "S1".into(),
            product: String::new(),
            bus: Bus::Usb,
            release: 0,
        });
        static OPENS: AtomicU32 = AtomicU32::new(0);
        static WRITES: LazyLock<Writes> = LazyLock::new(Arc::default);
        fn open() -> Result<Box<dyn Raw>, String> {
            OPENS.fetch_add(1, Ordering::Relaxed);
            let device = port::device_for(&INFO).unwrap();
            Ok(Box::new(Wire {
                device,
                writes: WRITES.clone(),
            }))
        }
        fn opener(_: &BTreeMap<String, usize>) -> (Vec<Box<dyn Backend>>, Vec<foxrgb_hid::Zone>) {
            let device = port::device_for(&INFO).unwrap();
            let lit = TwoPart(Port::open(&INFO, open).unwrap());
            (vec![port::guard(Box::new(lit), device)], Vec::new())
        }
        port::share(&[(INFO.vid, INFO.pid)]);
        // The service lists the fans first, as `hardware` does.
        let mut fans =
            foxrgb_xpack::fan::backend_on(&INFO, Cooled(Port::open(&INFO, open).unwrap()));
        let mut host = host("combined");
        host.hub_opener = opener;
        assert_eq!(
            reply(&host, r#"{"cmd":"hub_lights"}"#)["devices"][0]["id"],
            "hid:combined"
        );
        assert_eq!(OPENS.load(Ordering::Relaxed), 1);

        let mut fan = fans.fans().remove(0);
        let lights = thread::spawn({
            let host = Arc::new(host);
            move || {
                for i in 0..40u8 {
                    let frame = json!({"device": "hid:combined", "colors": format!("{i:02x}0000")});
                    let r = reply(
                        &host,
                        &json!({"cmd": "set_hub_lights", "frames": [frame]}).to_string(),
                    );
                    assert_eq!(r["failed"], json!({}));
                }
            }
        });
        for _ in 0..40 {
            fan.set_duty(0.5).unwrap();
        }
        lights.join().unwrap();
        let writes = locked(&WRITES).clone();
        assert_eq!(writes.len(), 160);
        assert!(writes.iter().all(|(_, held)| *held), "{writes:?}");
        for pair in writes.chunks(2) {
            assert!(
                matches!(pair, [(1, _), (2, _)] | [(3, _), (4, _)]),
                "{writes:?}"
            );
        }
    }
}
