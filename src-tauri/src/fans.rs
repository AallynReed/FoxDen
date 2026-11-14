//! The app's link to the background service: fan state, curves, and a service of the app's own.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use foxrgb_client::{Client, PIPE, Service, server_pid};
use foxrgb_fan::FanConfig;
use tauri::plugin::Builder;
use tauri::{AppHandle, Emitter, RunEvent, Wry};

use crate::config;

pub use foxrgb_client::FanState;

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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the worker that polls the service and emits "fans".
pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    // The only way to hear about the app quitting from here.
    let on_quit = Builder::<Wry>::new("fans").on_event(|_, event| {
        if let RunEvent::Exit = event {
            stop_child();
        }
    });
    if let Err(e) = app.plugin(on_quit.build()) {
        eprintln!("fans: {e}");
    }
    thread::spawn(|| {
        loop {
            poll(false);
            thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Asks the service how things are and tells the UI when that changed, or always with `force`.
fn poll(force: bool) -> FanState {
    let mut told = lock(&LINK.told);
    let state = LINK.client.state();
    if force || *told != state {
        *told = state.clone();
        if let Some(app) = APP.get() {
            let _ = app.emit("fans", &state);
        }
    }
    state
}

/// Sends the UI the state a fan command left behind, then hands on its result.
fn then_tell(result: Result<(), String>) -> Result<(), String> {
    poll(true);
    result
}

#[tauri::command(async)]
pub fn get_fans() -> Result<FanState, String> {
    Ok(poll(false))
}

#[tauri::command(async)]
pub fn set_fan_config(config: FanConfig) -> Result<(), String> {
    then_tell(LINK.client.set_config(&config))
}

#[tauri::command(async)]
pub fn set_fan_sim(on: bool) -> Result<(), String> {
    then_tell(LINK.client.set_sim(on))
}

#[tauri::command(async)]
pub fn start_service() -> Result<(), String> {
    then_tell(start_child())
}

#[tauri::command(async)]
pub fn stop_service() -> Result<(), String> {
    stop_child();
    match poll(true).service {
        Service::Stopped => Ok(()),
        _ => Err("this service was not started by FoxRGB, so FoxRGB leaves it running".into()),
    }
}

/// Starts foxrgb-service in this user session unless one already answers.
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

    let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("foxrgb-service.exe");
    let mut service = Command::new(&exe)
        .arg("run")
        .env("FOXRGB_DATA_DIR", config::dir().join("service"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;

    let deadline = Instant::now() + STARTUP;
    while LINK.client.status().is_err() {
        if let Ok(Some(status)) = service.try_wait() {
            return Err(format!("the service stopped as soon as it started ({status})"));
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
    let started: Option<u32> = fs::read_to_string(record()).ok().and_then(|id| id.trim().parse().ok());
    started.is_some() && started == server_pid(PIPE)
}

/// Stops the service this app started, if there is one.
fn stop_child() {
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
