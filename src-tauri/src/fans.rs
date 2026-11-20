//! The app's link to the background service: fan state, curves, and a service of the app's own.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use foxden_client::{Client, PIPE, Service, server_pid};
use foxden_fan::FanConfig;
use tauri::plugin::Builder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Wry};

use crate::config;
use crate::state::{Job, Jobs, Kinds, Shared, lock as lock_state};

pub use foxden_client::FanState;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Listing the hardware can keep the service busy for a few seconds.
const TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP: Duration = Duration::from_secs(15);
const EXIT: Duration = Duration::from_secs(5);

struct Link {
    client: Client,
    /// The state the UI was last told about.
    told: Mutex<FanState>,
    /// The service this app started, if any.
    child: Mutex<Option<Child>>,
}

static LINK: LazyLock<Link> = LazyLock::new(|| Link {
    client: Client::new(PIPE, TIMEOUT),
    told: Mutex::default(),
    child: Mutex::default(),
});
static APP: OnceLock<AppHandle> = OnceLock::new();
/// The Fans module switch.
static ON: AtomicBool = AtomicBool::new(true);
static PENDING: Mutex<Pending> = Mutex::new(Pending::Nothing);

/// Work owed to the service once it answers.
#[derive(Clone, Debug, PartialEq)]
enum Pending {
    Nothing,
    /// Keep the user's config, then hand every fan back to its firmware.
    Blank,
    Restore(FanConfig),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the worker that polls the service and emits "fans".
pub fn start(app: &AppHandle, on: bool, backup: Option<FanConfig>) {
    let _ = APP.set(app.clone());
    set_module(on, backup);
    // The only way to hear about the app quitting from here.
    let on_quit = Builder::<Wry>::new("fans").on_event(|_, event| {
        if let RunEvent::Exit = event {
            gpu_fan(None);
            stop_child();
        }
    });
    if let Err(e) = app.plugin(on_quit.build()) {
        crate::log::warn("fans", e.to_string());
    }
    thread::spawn(|| {
        // A record that names no running service is from a run that was killed after its service went.
        if !left_behind() {
            let _ = fs::remove_file(record());
        }
        loop {
            settle();
            if !ON.load(Ordering::Relaxed) {
                hubs_follow(tick(false, &LINK.client).1);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            let state = poll(false);
            gpu_fan(
                state
                    .fans
                    .iter()
                    .find(|f| f.id == "gpu0/fan")
                    .and_then(|f| f.duty),
            );
            thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Switches the Fans module. Off keeps the config and blanks the service; on puts `backup` back.
pub fn set_module(on: bool, backup: Option<FanConfig>) {
    ON.store(on, Ordering::Relaxed);
    *lock(&PENDING) = match (on, backup) {
        (false, _) => Pending::Blank,
        (true, Some(backup)) => Pending::Restore(backup),
        (true, None) => Pending::Nothing,
    };
    if !on {
        gpu_fan(None);
    }
}

/// Does the pending work once the service is there to take it.
pub fn settle() {
    let pending = lock(&PENDING).clone();
    let done = match &pending {
        Pending::Nothing => return,
        Pending::Blank => blank(),
        Pending::Restore(config) => {
            let ok = LINK.client.set_config(config).is_ok();
            if ok {
                keep_backup(None);
            }
            ok
        }
    };
    let mut slot = lock(&PENDING);
    if done && *slot == pending {
        *slot = Pending::Nothing;
    }
}

fn blank() -> bool {
    let Ok(config) = LINK.client.get_config() else {
        return false;
    };
    let empty = FanConfig::default();
    if config != empty {
        keep_backup(Some(config));
    }
    LINK.client.set_config(&empty).is_ok()
}

fn keep_backup(config: Option<FanConfig>) {
    if let Some(app) = APP.get() {
        let shared = app.state::<Shared>();
        let mut state = lock_state(&shared);
        state.config.fan_backup = config;
        state.changed = Some(Instant::now());
    }
}

/// Asks the service how things are and tells the UI when that changed, or always with `force`.
fn poll(force: bool) -> FanState {
    let mut told = lock(&LINK.told);
    let (state, up) = tick(true, &LINK.client);
    let state = state.unwrap_or_default();
    hubs_follow(up);
    if force || *told != state {
        *told = state.clone();
        if let Some(app) = APP.get() {
            let _ = app.emit("fans", &state);
        }
    }
    state
}

/// What the worker asks the service.
trait Pipe {
    fn answers(&self) -> bool;
    fn state(&self) -> FanState;
}

impl Pipe for Client {
    fn answers(&self) -> bool {
        self.status().is_ok()
    }
    fn state(&self) -> FanState {
        Client::state(self)
    }
}

/// One worker tick: the fans while the module is on, and whether a service answers, either way.
fn tick(on: bool, pipe: &impl Pipe) -> (Option<FanState>, bool) {
    if !on {
        return (None, pipe.answers());
    }
    let state = pipe.state();
    let up = match state.service {
        Service::Running => true,
        Service::Error => pipe.answers(),
        Service::Stopped => false,
    };
    (Some(state), up)
}

/// Moves the fan hubs' lighting between app and service when the service comes or goes.
fn hubs_follow(up: bool) {
    if crate::service_lights::service_changed(up)
        && let Some(jobs) = APP.get().and_then(|app| app.try_state::<Jobs>())
    {
        // Memory comes through the service; USB devices only move when a hub it owns is plugged in.
        let kinds = Kinds {
            hid: crate::service_lights::hubs_present(),
            board: true,
            ..Kinds::default()
        };
        let _ = jobs.send(Job::Reload(kinds));
    }
}

/// Sends the UI the state a fan command left behind, then hands on its result.
fn then_tell(result: Result<(), String>) -> Result<(), String> {
    poll(true);
    result
}

const OFF: &str = "the Fans module is off";

#[tauri::command(async)]
pub fn get_fans() -> Result<FanState, String> {
    if !ON.load(Ordering::Relaxed) {
        return Ok(FanState::default());
    }
    Ok(poll(false))
}

#[tauri::command(async)]
pub fn set_fan_config(config: FanConfig) -> Result<(), String> {
    if !ON.load(Ordering::Relaxed) {
        return Err(OFF.into());
    }
    then_tell(LINK.client.set_config(&config))
}

#[tauri::command(async)]
pub fn start_service() -> Result<(), String> {
    if !ON.load(Ordering::Relaxed) {
        return Err(OFF.into());
    }
    then_tell(start_child())
}

#[tauri::command(async)]
pub fn stop_service() -> Result<(), String> {
    stop_child();
    match poll(true).service {
        Service::Stopped => Ok(()),
        _ => Err("this service was not started by FoxDen, so FoxDen leaves it running".into()),
    }
}

/// Starts foxden-service in this user session unless one already answers.
fn start_child() -> Result<(), String> {
    let mut child = lock(&LINK.child);
    let state = LINK.client.state();
    match state.service {
        Service::Running => return Ok(()),
        Service::Error => return Err(state.detail.unwrap_or_default()),
        Service::Stopped => {}
    }
    // One started earlier that no longer answers is on its way out.
    if let Some(mut old) = child.take() {
        finish(&mut old);
    }

    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("foxden-service.exe");
    // Told who started it, the service hands the fans back and exits even when the app is killed.
    let mut service = Command::new(&exe)
        .args(["run", "--parent", &std::process::id().to_string()])
        .env("FOXDEN_DATA_DIR", config::dir().join("service"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;

    let deadline = Instant::now() + STARTUP;
    while LINK.client.status().is_err() {
        if let Ok(Some(status)) = service.try_wait() {
            return Err(format!(
                "the service stopped as soon as it started ({status})"
            ));
        }
        if Instant::now() > deadline {
            let _ = service.kill();
            let _ = service.wait();
            return Err("the service was started but does not answer".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = fs::write(record(), service.id().to_string());
    *child = Some(service);
    Ok(())
}

/// Keeps the process id of the service this app started, so a later run of the app still knows it.
fn record() -> PathBuf {
    config::dir().join("service.pid")
}

/// True while the pipe is served by a service that an earlier run of the app started.
fn left_behind() -> bool {
    let started: Option<u32> = fs::read_to_string(record())
        .ok()
        .and_then(|id| id.trim().parse().ok());
    started.is_some() && started == server_pid(PIPE)
}

/// Stops the service this app started, if there is one.
pub fn stop_child() {
    let mut slot = lock(&LINK.child);
    match slot.take() {
        Some(mut child) => {
            // While it runs it holds the pipe, so the request cannot reach another service.
            if let Ok(None) = child.try_wait() {
                let _ = LINK.client.shutdown();
            }
            finish(&mut child);
        }
        // A run of the app that was killed could not stop its service.
        None if left_behind() => {
            let _ = LINK.client.shutdown();
            let deadline = Instant::now() + EXIT;
            while left_behind() {
                if Instant::now() > deadline {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        None => return,
    }
    let _ = fs::remove_file(record());
}

/// Gives the service time to exit by itself and ends it only when it does not.
fn finish(child: &mut Child) {
    let deadline = Instant::now() + EXIT;
    while let Ok(None) = child.try_wait() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Applies the service's duty for the graphics card fan, or gives the card back to its driver.
fn gpu_fan(duty: Option<f32>) {
    static CARD: Mutex<Option<Box<dyn foxden_fan::Fan>>> = Mutex::new(None);
    let mut card = lock(&CARD);
    match duty {
        Some(duty) => {
            if card.is_none() {
                *card = foxden_adlx::fan().ok();
            }
            if let Some(fan) = card.as_mut() {
                let _ = fan.set_duty(duty);
            }
        }
        None => {
            if let Some(fan) = card.as_mut() {
                let _ = fan.release();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fake {
        answers: bool,
        service: Service,
        asked_state: Cell<bool>,
    }

    impl Pipe for Fake {
        fn answers(&self) -> bool {
            self.answers
        }
        fn state(&self) -> FanState {
            self.asked_state.set(true);
            FanState {
                service: self.service,
                ..FanState::default()
            }
        }
    }

    fn fake(answers: bool, service: Service) -> Fake {
        Fake {
            answers,
            service,
            asked_state: Cell::new(false),
        }
    }

    // The hubs' lighting follows the service with the Fans module off, without asking for fans.
    #[test]
    fn the_service_is_noticed_with_the_fans_module_off() {
        let up = fake(true, Service::Running);
        assert_eq!(tick(false, &up), (None, true));
        assert!(!up.asked_state.get());
        assert_eq!(tick(false, &fake(false, Service::Stopped)), (None, false));
        let (state, seen) = tick(true, &up);
        assert!(seen && state.is_some() && up.asked_state.get());
        assert!(!tick(true, &fake(false, Service::Stopped)).1);
        // A service that answers but fails a config read still owns the hubs, as `hubs` sees it.
        assert!(tick(true, &fake(true, Service::Error)).1);
        assert!(!tick(true, &fake(false, Service::Error)).1);
    }
}
