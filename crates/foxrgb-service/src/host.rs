use std::fs;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use foxrgb_fan::{Controller, FanBackend, FanConfig, Status};
use serde::Deserialize;
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

use crate::pipe;

pub const MAX_LINE: usize = 64 * 1024;
const LOG_CAP: u64 = 1024 * 1024;

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

pub fn load_config(dir: &Path) -> Result<FanConfig, String> {
    let path = dir.join("fans.json");
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FanConfig::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn save_config(dir: &Path, config: &FanConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let path = dir.join("fans.json");
    let tmp = dir.join("fans.json.tmp");
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    };
    write().map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Status,
    GetConfig,
    SetConfig { config: FanConfig },
}

struct State {
    controller: Controller,
    status: Status,
}

pub struct Host {
    pub dir: PathBuf,
    state: Mutex<State>,
    /// True while the config in force is not the one on disk.
    unsaved: Mutex<bool>,
    stop: AtomicBool,
}

impl Host {
    pub fn new(dir: PathBuf, backends: Vec<Box<dyn FanBackend>>) -> Host {
        let mut controller = Controller::new(backends, FanConfig::default());
        if let Err(e) = load_config(&dir).and_then(|config| controller.set_config(config)) {
            log(
                &dir,
                &format!("ignoring fans.json, running with an empty config: {e}"),
            );
        }
        let state = State {
            controller,
            status: Status::default(),
        };
        Host {
            dir,
            state: Mutex::new(state),
            unsaved: Mutex::new(false),
            stop: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Answers one request line with one reply line.
    pub fn handle(&self, line: &str) -> String {
        if line.len() > MAX_LINE {
            return fail(format!("line longer than {MAX_LINE} bytes")).to_string();
        }
        let reply = match serde_json::from_str(line) {
            Ok(Request::Status) => json!({
                "ok": true,
                "status": self.lock().status,
                "version": env!("CARGO_PKG_VERSION"),
            }),
            Ok(Request::GetConfig) => {
                json!({"ok": true, "config": self.lock().controller.config()})
            }
            Ok(Request::SetConfig { config }) => match self.set_config(config) {
                Ok(()) => json!({"ok": true}),
                Err(e) => fail(e),
            },
            Err(e) => fail(format!("bad request: {e}")),
        };
        reply.to_string()
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

    fn control_loop(&self) {
        let mut last = Instant::now();
        while !self.stopping() {
            let now = Instant::now();
            {
                let mut state = self.lock();
                let status = state.controller.tick((now - last).as_secs_f32());
                state.status = status;
            }
            last = now;
            while !self.stopping() && now.elapsed() < Duration::from_secs(1) {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    pub fn release_all(&self) {
        let mut state = self.lock();
        state.controller.release_all();
        let fans = &mut state.status.fans;
        let driven = fans.iter().filter(|fan| fan.duty.is_some()).count();
        fans.iter_mut().for_each(|fan| fan.duty = None);
        log(
            &self.dir,
            &format!("released fans to firmware control ({driven} had a duty set)"),
        );
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
    backends: Vec<Box<dyn FanBackend>>,
    pipe_name: &str,
) -> Result<Running, String> {
    // Taking the pipe name first keeps a second instance away from the fans.
    let first = pipe::create(pipe_name, true)
        .map_err(|e| format!("{e} (is foxrgb-service already running?)"))?;
    let host = Arc::new(Host::new(dir, backends));
    let version = env!("CARGO_PKG_VERSION");
    log(
        &host.dir,
        &format!("started, version {version}, listening on {pipe_name}"),
    );

    let h = host.clone();
    let control = thread::spawn(move || {
        if catch_unwind(AssertUnwindSafe(|| h.control_loop())).is_err() {
            h.release_all();
            log(&h.dir, "control loop panicked, fans stay with the firmware");
        }
    });
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
    use foxrgb_fan::{Curve, Fan, FanRule, Sensor, SimBackend};

    use super::*;

    /// A backend with one fan that records its release.
    #[derive(Clone)]
    struct Probe {
        released: Arc<AtomicBool>,
        panics: bool,
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
            assert!(!self.panics, "the probe fan panics on purpose");
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> {
            self.released.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    fn start_probe(name: &str, panics: bool) -> (Running, Arc<AtomicBool>) {
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
        let released = Arc::new(AtomicBool::new(false));
        let probe = Probe {
            released: released.clone(),
            panics,
        };
        let pipe = format!(
            r"\\.\pipe\foxrgb-service-test-{name}-{}",
            std::process::id()
        );
        let running = start(dir, vec![Box::new(probe)], &pipe).unwrap();
        (running, released)
    }

    pub fn wait_for(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn stop_releases_a_driven_fan() {
        let (running, released) = start_probe("release", false);
        wait_for("the first duty", || {
            let state = running.host.lock();
            state.status.fans.iter().any(|fan| fan.duty.is_some())
        });
        assert!(!released.load(Ordering::Relaxed));
        running.stop();
        assert!(released.load(Ordering::Relaxed));
    }

    #[test]
    fn dropping_the_host_releases_a_driven_fan() {
        let (running, released) = start_probe("drop", false);
        wait_for("the first duty", || {
            let state = running.host.lock();
            state.status.fans.iter().any(|fan| fan.duty.is_some())
        });
        drop(running);
        assert!(released.load(Ordering::Relaxed));
    }

    #[test]
    fn a_panic_in_the_control_loop_releases_the_fans() {
        let (running, released) = start_probe("panic", true);
        wait_for("the release", || released.load(Ordering::Relaxed));
        assert_eq!(reply(&running.host, r#"{"cmd":"status"}"#)["ok"], true);
        running.stop();
    }

    pub fn scratch(name: &str) -> PathBuf {
        // Beside the test binary by default, so nothing is left in the system temp folder.
        let beside = || std::env::current_exe().unwrap().with_file_name("foxrgb-service-tests");
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

    fn host(name: &str) -> Host {
        Host::new(scratch(name), vec![Box::<SimBackend>::default()])
    }

    fn reply(host: &Host, line: &str) -> Value {
        serde_json::from_str(&host.handle(line)).unwrap()
    }

    #[test]
    fn status_reports_version() {
        let r = reply(&host("status"), r#"{"cmd":"status"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["version"], env!("CARGO_PKG_VERSION"));
        assert!(r["status"]["fans"].is_array());
        assert!(r["status"]["sensors"].is_array());
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
        ];
        for line in lines {
            let r = reply(&host, line);
            assert_eq!(r["ok"], false, "{line}");
            assert!(r["error"].is_string(), "{line}");
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
        let host = Host::new(dir, Vec::new());
        assert_eq!(*host.lock().controller.config(), sample());
    }

    #[test]
    fn invalid_file_means_empty_config_and_a_log_line() {
        let dir = scratch("invalid");
        fs::write(dir.join("fans.json"), "{ nope").unwrap();
        let host = Host::new(dir.clone(), Vec::new());
        assert_eq!(*host.lock().controller.config(), FanConfig::default());
        assert!(
            fs::read_to_string(dir.join("service.log"))
                .unwrap()
                .contains("fans.json")
        );
    }
}
