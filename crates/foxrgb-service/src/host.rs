use std::any::Any;
use std::fs;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use foxrgb_fan::{Controller, FanBackend, FanConfig, SimBackend, Status};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::SYSTEMTIME;
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

/// Lists the hardware backends: at start, and again whenever the simulated one is switched.
type Hardware = Box<dyn Fn() -> Vec<Box<dyn FanBackend>> + Send + Sync>;

#[derive(Clone, Copy)]
pub enum Mode {
    Console,
    Service,
}

pub fn data_dir() -> PathBuf {
    match std::env::var_os("FOXRGB_DATA_DIR") {
        Some(dir) if !dir.is_empty() => dir.into(),
        _ => {
            let base = std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into());
            PathBuf::from(base).join("FoxRGB")
        }
    }
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
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// A missing file reads as the default value.
fn load<T: DeserializeOwned + Default>(dir: &Path, file: &str) -> Result<T, String> {
    let path = dir.join(file);
    match fs::read_to_string(&path) {
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
        let mut file = fs::File::create(&tmp)?;
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

/// What the service remembers besides the fan config, in service.json.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    sim: bool,
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Status,
    GetConfig,
    SetConfig { config: FanConfig },
    SetSim { on: bool },
    Shutdown,
}

struct State {
    controller: Controller,
    status: Status,
    sim: bool,
}

pub struct Host {
    pub dir: PathBuf,
    hardware: Hardware,
    mode: Mode,
    state: Mutex<State>,
    /// True while the config in force is not the one on disk. Held by every request that
    /// writes a file, which keeps those writes in order.
    unsaved: Mutex<bool>,
    stop: AtomicBool,
    /// Why the process should end: a shutdown request, or a control loop that keeps panicking.
    ended: Mutex<Option<Result<(), String>>>,
}

impl Host {
    /// `sim` starts with simulated fans even if the last run switched them off.
    pub fn new(
        dir: PathBuf,
        hardware: impl Fn() -> Vec<Box<dyn FanBackend>> + Send + Sync + 'static,
        sim: bool,
        mode: Mode,
    ) -> Host {
        let settings = load(&dir, "service.json").unwrap_or_else(|e| {
            log(&dir, &format!("ignoring service.json: {e}"));
            Settings::default()
        });
        let sim = sim || settings.sim;
        let hardware: Hardware = Box::new(hardware);
        let mut controller = Controller::new(backends(&hardware, sim), FanConfig::default());
        if let Err(e) = load_config(&dir).and_then(|config| controller.set_config(config)) {
            log(
                &dir,
                &format!("ignoring fans.json, running with an empty config: {e}"),
            );
        }
        let state = State {
            controller,
            status: Status::default(),
            sim,
        };
        Host {
            dir,
            hardware,
            mode,
            state: Mutex::new(state),
            unsaved: Mutex::new(false),
            stop: AtomicBool::new(false),
            ended: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Asks the process to end. The first reason given is the one that counts.
    pub fn end(&self, outcome: Result<(), String>) {
        let mut ended = self.ended.lock().unwrap_or_else(|e| e.into_inner());
        ended.get_or_insert(outcome);
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
                    "sim": state.sim,
                    "status": state.status,
                    "version": env!("CARGO_PKG_VERSION"),
                })
            }
            Ok(Request::GetConfig) => {
                json!({"ok": true, "config": self.lock().controller.config()})
            }
            Ok(Request::SetConfig { config }) => done(self.set_config(config)),
            Ok(Request::SetSim { on }) => done(self.set_sim(on)),
            Ok(Request::Shutdown) => match self.mode {
                Mode::Console => {
                    shutdown = true;
                    json!({"ok": true})
                }
                Mode::Service => fail(NO_SHUTDOWN.into()),
            },
            Err(e) => fail(format!("bad request: {e}")),
        };
        (reply.to_string(), shutdown)
    }

    fn set_config(&self, config: FanConfig) -> Result<(), String> {
        // Orders the file writes. The control loop never takes this lock.
        let mut unsaved = self.unsaved.lock().unwrap_or_else(|e| e.into_inner());
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

    fn set_sim(&self, on: bool) -> Result<(), String> {
        // Orders the file writes, as in set_config.
        let _turn = self.unsaved.lock().unwrap_or_else(|e| e.into_inner());
        let switched = {
            // Locked throughout, so the control loop waits while the hardware is listed again.
            let mut state = self.lock();
            // A switch drives the fans again, which must not happen after the final release.
            if self.stopping() {
                return Err("the service is stopping".into());
            }
            if state.sim == on {
                None
            } else {
                let fans = &state.status.fans;
                let driven = fans.iter().filter(|fan| fan.duty.is_some()).count();
                // The old backends are released and gone before new ones are opened.
                let emptied = state.controller.set_backends(Vec::new());
                emptied.map_err(|e| format!("not switched, a fan could not be released: {e}"))?;
                let backends = backends(&self.hardware, on);
                state.controller.set_backends(backends)?;
                state.sim = on;
                state.status = state.controller.tick(0.0);
                Some(driven)
            }
        };
        // Written with the state unlocked: a stuck console or disk must not stall the fans.
        if let Some(driven) = switched {
            let now = if on { "on" } else { "off" };
            let released = format!("fans released first ({driven} had a duty set)");
            log(
                &self.dir,
                &format!("simulated fans switched {now}, {released}"),
            );
        }
        save(&self.dir, "service.json", &Settings { sim: on })
            .map_err(|e| format!("choice applied but not saved: {e}"))
    }

    fn tick(&self, dt: f32) {
        let mut state = self.lock();
        state.status = state.controller.tick(dt);
    }

    fn control_loop(&self) {
        let mut last = Instant::now();
        while !self.stopping() {
            let now = Instant::now();
            self.tick((now - last).as_secs_f32());
            last = now;
            while !self.stopping() && now.elapsed() < Duration::from_secs(1) {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    /// Runs the control loop, starting it again after a panic until the back-off runs out.
    fn supervise(&self) {
        let mut waits = BACK_OFF.iter();
        loop {
            let Err(panic) = catch_unwind(AssertUnwindSafe(|| self.control_loop())) else {
                return;
            };
            let why = panic_text(&*panic);
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
        let mut state = self.lock();
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

fn backends(hardware: &Hardware, sim: bool) -> Vec<Box<dyn FanBackend>> {
    let mut backends = hardware();
    if sim {
        backends.push(Box::new(SimBackend::swinging()));
    }
    backends
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
    pipe: JoinHandle<()>,
}

/// Starts the control loop and the pipe server.
pub fn start(
    dir: PathBuf,
    hardware: impl Fn() -> Vec<Box<dyn FanBackend>> + Send + Sync + 'static,
    sim: bool,
    mode: Mode,
    pipe_name: &str,
) -> Result<Running, String> {
    // Taking the pipe name first keeps a second instance away from the fans.
    let first = pipe::create(pipe_name, true)
        .map_err(|e| format!("{e} (is foxrgb-service already running?)"))?;
    let host = Arc::new(Host::new(dir, hardware, sim, mode));
    let version = env!("CARGO_PKG_VERSION");
    let sim = if host.lock().sim { "on" } else { "off" };
    log(
        &host.dir,
        &format!("started, version {version}, listening on {pipe_name}, simulated fans {sim}"),
    );

    let h = host.clone();
    let control = thread::spawn(move || h.supervise());
    let (h, name) = (host.clone(), pipe_name.to_string());
    let pipe = thread::spawn(move || pipe::serve(&h, &name, first));
    Ok(Running {
        host,
        pipe_name: pipe_name.to_string(),
        control: Some(control),
        pipe,
    })
}

impl Running {
    /// Set once the process should end: `Ok` after a shutdown request, `Err` when the control
    /// loop has given up.
    pub fn ended(&self) -> Option<Result<(), String>> {
        let ended = self.host.ended.lock().unwrap_or_else(|e| e.into_inner());
        ended.clone()
    }

    /// Stops both threads and hands the fans back. Dropping does the same.
    pub fn stop(self) {}
}

impl Drop for Running {
    fn drop(&mut self) {
        self.host.stop.store(true, Ordering::Relaxed);
        // Fans first: the pipe shutdown below must not be able to delay the release.
        if let Some(control) = self.control.take() {
            let _ = control.join();
        }
        self.host.release_all();
        while !self.pipe.is_finished() {
            pipe::wake(&self.pipe_name);
            thread::sleep(Duration::from_millis(20));
        }
        log(&self.host.dir, "stopped");
    }
}

#[cfg(test)]
pub(crate) mod tests {
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
        start(dir, hardware(probe), false, Mode::Console, &pipe).unwrap()
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

    /// A console host with simulated fans and no hardware.
    fn host(name: &str) -> Host {
        Host::new(scratch(name), Vec::new, true, Mode::Console)
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
    fn status_reports_version_and_sim() {
        let r = reply(&host("status"), r#"{"cmd":"status"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(r["sim"], true);
        assert!(r["status"]["fans"].is_array());
        assert!(r["status"]["sensors"].is_array());

        let plain = Host::new(scratch("status-plain"), Vec::new, false, Mode::Console);
        assert_eq!(reply(&plain, r#"{"cmd":"status"}"#)["sim"], false);
    }

    #[test]
    fn set_sim_adds_and_removes_the_simulated_backend() {
        let host = Host::new(scratch("sim"), Vec::new, false, Mode::Console);
        host.tick(1.0);
        assert!(fan_ids(&host).is_empty());

        let on = r#"{"cmd":"set_sim","on":true}"#;
        assert_eq!(reply(&host, on), json!({"ok": true}));
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], true);
        assert!(fan_ids(&host).contains(&"sim/cpu_fan".to_string()));
        assert_eq!(reply(&host, on), json!({"ok": true}));

        let off = r#"{"cmd":"set_sim","on":false}"#;
        assert_eq!(reply(&host, off), json!({"ok": true}));
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], false);
        assert!(fan_ids(&host).is_empty());
    }

    #[test]
    fn the_sim_choice_is_remembered() {
        let dir = scratch("remember");
        let host = Host::new(dir.clone(), Vec::new, false, Mode::Console);
        reply(&host, r#"{"cmd":"set_sim","on":true}"#);
        drop(host);
        assert!(!dir.join("service.json.tmp").exists());
        let host = Host::new(dir.clone(), Vec::new, false, Mode::Service);
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], true);

        reply(&host, r#"{"cmd":"set_sim","on":false}"#);
        drop(host);
        let host = Host::new(dir.clone(), Vec::new, false, Mode::Console);
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], false);
        // `run --sim` wins over the remembered choice without changing it.
        drop(host);
        let host = Host::new(dir.clone(), Vec::new, true, Mode::Console);
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], true);
        assert!(!load::<Settings>(&dir, "service.json").unwrap().sim);
    }

    #[test]
    fn a_broken_settings_file_means_no_sim_and_a_log_line() {
        let dir = scratch("settings");
        fs::write(dir.join("service.json"), "{ nope").unwrap();
        let host = Host::new(dir.clone(), Vec::new, false, Mode::Console);
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], false);
        assert!(log_of(&dir).contains("service.json"));
    }

    #[test]
    fn switching_the_sim_releases_driven_fans_first() {
        let probe = Probe::default();
        // Whether the fan had been released each time the hardware was listed.
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hardware = {
            let (probe, seen) = (probe.clone(), seen.clone());
            move || {
                let released = probe.released.load(Ordering::Relaxed);
                seen.lock().unwrap().push(released);
                vec![Box::new(probe.clone()) as Box<dyn FanBackend>]
            }
        };
        let host = Host::new(probe_dir("swap"), hardware, false, Mode::Console);
        host.tick(1.0);
        assert!(driven(&host));

        let on = r#"{"cmd":"set_sim","on":true}"#;
        assert_eq!(reply(&host, on), json!({"ok": true}));
        assert_eq!(*seen.lock().unwrap(), [false, true]);
        // The hardware's fan is driven as before.
        let ids = fan_ids(&host);
        assert_eq!(ids.len(), 4, "{ids:?}");
        assert_eq!(ids[0], "probe");
        assert!(driven(&host));
        assert!(log_of(&host.dir).contains("fans released first (1 had a duty set)"));
    }

    #[test]
    fn the_sim_is_not_switched_while_a_fan_refuses_release() {
        let probe = Probe {
            stuck: true,
            ..Probe::default()
        };
        let host = Host::new(probe_dir("stuck"), hardware(&probe), false, Mode::Console);
        host.tick(1.0);
        let r = reply(&host, r#"{"cmd":"set_sim","on":true}"#);
        assert_eq!(r["ok"], false);
        let error = r["error"].as_str().unwrap();
        assert!(error.ends_with("probe: stuck"), "{error}");
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], false);
        assert_eq!(fan_ids(&host), ["probe"]);
        assert!(!host.dir.join("service.json").exists());
    }

    #[test]
    fn a_stopping_host_switches_nothing() {
        let probe = Probe::default();
        let host = Host::new(probe_dir("late"), hardware(&probe), false, Mode::Console);
        host.tick(1.0);
        assert!(driven(&host));
        host.stop.store(true, Ordering::Relaxed);
        host.release_all();
        assert!(!driven(&host));

        // A client that is still connected may ask after the fans were handed back.
        let r = reply(&host, r#"{"cmd":"set_sim","on":true}"#);
        assert_eq!(r["ok"], false);
        assert!(r["error"].is_string());
        assert!(!driven(&host));
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], false);
        assert!(!host.dir.join("service.json").exists());
    }

    #[test]
    fn shutdown_is_for_console_mode_only() {
        let console = host("shutdown");
        let (line, shutdown) = console.handle(r#"{"cmd":"shutdown"}"#);
        assert_eq!(line, r#"{"ok":true}"#);
        assert!(shutdown);

        let service = Host::new(scratch("shutdown-service"), Vec::new, false, Mode::Service);
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
    fn a_sim_choice_that_was_not_saved_is_saved_by_the_next_request() {
        let host = Host::new(scratch("sim-retry"), Vec::new, false, Mode::Console);
        let blocker = host.dir.join("service.json");
        fs::create_dir(&blocker).unwrap();
        let on = r#"{"cmd":"set_sim","on":true}"#;
        assert_eq!(reply(&host, on)["ok"], false);
        assert_eq!(reply(&host, r#"{"cmd":"status"}"#)["sim"], true);
        fs::remove_dir(&blocker).unwrap();
        assert_eq!(reply(&host, on), json!({"ok": true}));
        assert!(load::<Settings>(&host.dir, "service.json").unwrap().sim);
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
            r#"{"cmd":"set_sim"}"#,
            r#"{"cmd":"set_sim","on":"yes"}"#,
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
        assert!(!host.dir.join("service.json").exists());
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
        let host = Host::new(dir, Vec::new, false, Mode::Console);
        assert_eq!(*host.lock().controller.config(), sample());
    }

    #[test]
    fn invalid_file_means_empty_config_and_a_log_line() {
        let dir = scratch("invalid");
        fs::write(dir.join("fans.json"), "{ nope").unwrap();
        let host = Host::new(dir.clone(), Vec::new, false, Mode::Console);
        assert_eq!(*host.lock().controller.config(), FanConfig::default());
        assert!(log_of(&dir).contains("fans.json"));
    }
}
