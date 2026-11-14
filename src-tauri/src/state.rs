use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use foxrgb_audio::AudioCapture;
use foxrgb_core::{Backend, DeviceKind, Effect, EffectInfo, Engine, Inputs, VirtualBackend};
use foxrgb_openrgb::OpenRgbBackend;
use foxrgb_screen::ScreenCapture;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::config::{self, Config, Settings};
use crate::tray;

const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Seconds after which effect time starts again from zero.
const CLOCK_WRAP: f64 = 21_600.0;

#[derive(Clone, Debug, Serialize)]
pub struct BackendStatus {
    pub name: String,
    pub active: bool,
    pub devices: usize,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct DeviceView {
    id: String,
    name: String,
    kind: DeviceKind,
    backend: String,
    rect: [f32; 4],
    grid: [u32; 2],
    leds: Vec<[f32; 2]>,
    enabled: bool,
    brightness: f32,
}

#[derive(Serialize)]
pub struct Snapshot {
    devices: Vec<DeviceView>,
    effect: Effect,
    brightness: f32,
    pub paused: bool,
    catalog: Vec<EffectInfo>,
    pub profiles: Vec<String>,
    pub active_profile: Option<String>,
    settings: Settings,
    backends: Vec<BackendStatus>,
}

/// Outcome of connecting one kind of backend: its name and the error, if it failed.
type Loaded = Vec<(&'static str, Option<String>)>;

pub struct AppState {
    /// Owns the live effect and brightness; `config` holds everything else.
    pub engine: Engine,
    pub config: Config,
    /// When the unsaved changes were last touched.
    pub changed: Option<Instant>,
    loaded: Loaded,
    inputs: Vec<BackendStatus>,
    scan: u64,
    retry_inputs: bool,
}

pub type Shared = Arc<Mutex<AppState>>;

pub fn lock(shared: &Shared) -> MutexGuard<'_, AppState> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let mut engine = Engine::new(Vec::new());
        engine.effect = config.effect.clone();
        engine.brightness = config.brightness;
        for (id, device) in &config.devices {
            engine.set_device(id, device.enabled, device.brightness, device.rect);
        }
        let mut loaded = Loaded::new();
        if config.settings.show_virtual {
            engine.set_backends(vec![("virtual".into(), Box::new(VirtualBackend))]);
            loaded.push(("virtual", None));
        }
        AppState { engine, config, changed: None, loaded, inputs: Vec::new(), scan: 0, retry_inputs: false }
    }

    pub fn snapshot(&self) -> Snapshot {
        let devices = self
            .engine
            .views()
            .map(|(backend, device, enabled, brightness)| DeviceView {
                id: device.id.clone(),
                name: device.name.clone(),
                kind: device.kind,
                backend: backend.to_string(),
                rect: device.rect,
                grid: device.grid,
                leds: device.leds.clone(),
                enabled,
                brightness,
            })
            .collect();
        Snapshot {
            devices,
            effect: self.engine.effect.clone(),
            brightness: self.engine.brightness,
            paused: self.engine.paused,
            catalog: Effect::catalog(),
            profiles: self.config.profiles.keys().cloned().collect(),
            active_profile: self.config.active_profile.clone(),
            settings: self.config.settings.clone(),
            backends: self.backends(),
        }
    }

    fn backends(&self) -> Vec<BackendStatus> {
        let slots = self.engine.status();
        let hardware = ["virtual", "openrgb", "hid"].into_iter().map(|name| {
            let mine = || slots.iter().filter(|(label, ..)| label == name);
            let outcome = self.loaded.iter().find(|(n, _)| *n == name);
            let error = outcome.and_then(|(_, e)| e.clone()).or_else(|| mine().find_map(|(.., e)| e.clone()));
            BackendStatus {
                name: name.into(),
                active: outcome.is_some() && error.is_none(),
                devices: mine().map(|(_, count, _)| count).sum(),
                error,
            }
        });
        hardware.chain(self.inputs.iter().cloned()).collect()
    }

    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        let profile = self.config.profiles.get(name).ok_or_else(|| format!("no profile named {name}"))?;
        self.engine.effect = profile.effect.clone();
        self.engine.brightness = profile.brightness.clamp(0.0, 1.0);
        self.config.active_profile = Some(name.to_string());
        Ok(())
    }

    fn to_save(&self) -> Config {
        Config {
            effect: self.engine.effect.clone(),
            brightness: self.engine.brightness,
            ..self.config.clone()
        }
    }
}

/// Applies `change`, schedules a save and tells the UI and the tray.
pub fn update(
    app: &AppHandle,
    shared: &Shared,
    change: impl FnOnce(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    {
        let mut state = lock(shared);
        change(&mut state)?;
        state.changed = Some(Instant::now());
    }
    emit_snapshot(app, shared);
    Ok(())
}

pub fn emit_snapshot(app: &AppHandle, shared: &Shared) {
    let snapshot = lock(shared).snapshot();
    let _ = app.emit("snapshot", &snapshot);
    tray::refresh(app, &snapshot);
}

/// Connects every enabled backend on a worker thread, then swaps them in.
pub fn reload_backends(app: &AppHandle, shared: &Shared) {
    let (settings, scan) = {
        let mut state = lock(shared);
        state.scan += 1;
        state.retry_inputs = true;
        (state.config.settings.clone(), state.scan)
    };
    let (app, shared) = (app.clone(), shared.clone());
    thread::spawn(move || {
        let (backends, loaded) = connect(&settings);
        {
            let mut state = lock(&shared);
            // A newer reload is on its way.
            if state.scan != scan {
                return;
            }
            state.engine.set_backends(backends);
            state.loaded = loaded;
        }
        emit_snapshot(&app, &shared);
    });
}

type Backends = Vec<(String, Box<dyn Backend>)>;

fn connect(settings: &Settings) -> (Backends, Loaded) {
    let mut backends = Backends::new();
    let mut loaded = Loaded::new();
    if settings.show_virtual {
        backends.push(("virtual".into(), Box::new(VirtualBackend)));
        loaded.push(("virtual", None));
    }
    if settings.openrgb_enabled {
        match OpenRgbBackend::connect(&settings.openrgb_host, settings.openrgb_port) {
            Ok(backend) => {
                backends.push(("openrgb".into(), Box::new(backend)));
                loaded.push(("openrgb", None));
            }
            Err(e) => loaded.push(("openrgb", Some(e))),
        }
    }
    if settings.native_drivers {
        backends.extend(foxrgb_hid::probe().into_iter().map(|b| ("hid".to_string(), b)));
        loaded.push(("hid", None));
    }
    (backends, loaded)
}

/// An audio or screen capture that only runs while the effect needs it.
struct Input<T> {
    capture: Option<T>,
    error: Option<String>,
}

impl<T> Input<T> {
    fn off() -> Self {
        Input { capture: None, error: None }
    }

    /// Starts or drops the capture to match `wanted`. True when its status changed.
    fn sync(&mut self, wanted: bool, retry: bool, start: fn() -> Result<T, String>) -> bool {
        if !wanted {
            let was_on = self.capture.is_some() || self.error.is_some();
            *self = Input::off();
            return was_on;
        }
        if self.capture.is_some() || (self.error.is_some() && !retry) {
            return false;
        }
        match start() {
            Ok(capture) => *self = Input { capture: Some(capture), error: None },
            Err(e) => self.error = Some(e),
        }
        true
    }

    fn status(&self, name: &str) -> Option<BackendStatus> {
        (self.capture.is_some() || self.error.is_some()).then(|| BackendStatus {
            name: name.into(),
            active: self.capture.is_some(),
            devices: 0,
            error: self.error.clone(),
        })
    }
}

pub fn spawn_render(app: AppHandle, shared: Shared) {
    thread::spawn(move || {
        let start = Instant::now();
        let mut next = start;
        let mut inputs = Inputs::default();
        let mut audio = Input::<AudioCapture>::off();
        let mut screen = Input::<ScreenCapture>::off();
        loop {
            let (fps, want_audio, want_screen, retry) = {
                let mut state = lock(&shared);
                let effect = &state.engine.effect;
                let wanted = (effect.uses_audio(), effect.uses_screen());
                (state.config.settings.fps, wanted.0, wanted.1, std::mem::take(&mut state.retry_inputs))
            };
            let audio_changed = audio.sync(want_audio, retry, AudioCapture::start);
            let screen_changed = screen.sync(want_screen, retry, ScreenCapture::start);
            inputs.audio = audio.capture.as_ref().map(|c| c.frame()).unwrap_or_default();
            inputs.screen = screen.capture.as_ref().map(|c| c.frame()).unwrap_or_default();

            let changed = audio_changed || screen_changed;
            let (frames, failed) = {
                let mut state = lock(&shared);
                if changed {
                    state.inputs = [audio.status("audio"), screen.status("screen")].into_iter().flatten().collect();
                }
                // f32 seconds turn coarse after days of uptime, so the clock wraps.
                let t = (start.elapsed().as_secs_f64() % CLOCK_WRAP) as f32;
                let frames = state.engine.render(t, &inputs);
                (frames, state.engine.take_failed())
            };
            let _ = app.emit("frame", &frames);
            if changed || failed {
                emit_snapshot(&app, &shared);
            }

            next += Duration::from_secs_f64(1.0 / f64::from(fps.clamp(10, 60)));
            match next.checked_duration_since(Instant::now()) {
                Some(wait) => thread::sleep(wait),
                None => next = Instant::now(),
            }
        }
    });
}

/// Writes the config once changes have settled for a moment.
pub fn spawn_saver(shared: Shared) {
    thread::spawn(move || {
        loop {
            thread::sleep(SAVE_DELAY / 2);
            let config = {
                let mut state = lock(&shared);
                if state.changed.is_none_or(|at| at.elapsed() < SAVE_DELAY) {
                    continue;
                }
                state.changed = None;
                state.to_save()
            };
            save(&config);
        }
    });
}

/// Last thing before exit: lights off, pending changes on disk.
pub fn shutdown(shared: &Shared) {
    let config = {
        let mut state = lock(shared);
        state.engine.paused = true;
        state.engine.blackout();
        state.changed.take().map(|_| state.to_save())
    };
    if let Some(config) = config {
        save(&config);
    }
}

fn save(config: &Config) {
    if let Err(e) = config::save(&config::dir(), config) {
        eprintln!("config: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn snapshot_has_the_shape_the_ui_expects() {
        let mut state = AppState::new(Config::default());
        state.engine.set_device("virtual:gpu", false, 0.5, Some([0.25, 0.5, 0.25, 0.125]));
        let json = serde_json::to_value(state.snapshot()).unwrap();
        assert_eq!(
            keys(&json),
            [
                "active_profile", "backends", "brightness", "catalog", "devices", "effect", "paused",
                "profiles", "settings"
            ]
        );
        assert_eq!(
            keys(&json["settings"]),
            [
                "autostart", "close_to_tray", "fps", "native_drivers", "openrgb_enabled", "openrgb_host",
                "openrgb_port", "show_virtual", "start_minimized"
            ]
        );
        assert_eq!(json["settings"]["native_drivers"], false);
        assert_eq!(json["settings"]["autostart"], false);

        let gpu = json["devices"].as_array().unwrap().iter().find(|d| d["id"] == "virtual:gpu").unwrap();
        assert_eq!(keys(gpu), ["backend", "brightness", "enabled", "grid", "id", "kind", "leds", "name", "rect"]);
        assert_eq!(gpu["backend"], "virtual");
        assert_eq!(gpu["kind"], "gpu");
        assert_eq!(gpu["enabled"], false);
        assert_eq!(gpu["brightness"], 0.5);
        assert_eq!(gpu["rect"], json!([0.25, 0.5, 0.25, 0.125]));
        assert_eq!(gpu["grid"], json!([12, 1]));

        let backends = json["backends"].as_array().unwrap();
        assert_eq!(backends[0], json!({ "name": "virtual", "active": true, "devices": 6, "error": null }));
        assert_eq!(backends[1], json!({ "name": "openrgb", "active": false, "devices": 0, "error": null }));
        assert_eq!(backends[2], json!({ "name": "hid", "active": false, "devices": 0, "error": null }));
    }

    #[test]
    fn loading_a_profile_replaces_effect_and_brightness() {
        let mut state = AppState::new(Config::default());
        let profile = config::Profile { effect: Effect::Cycle { speed: 2.0 }, brightness: 0.25 };
        state.config.profiles.insert("Night".into(), profile);
        assert!(state.load_profile("Day").is_err());
        state.load_profile("Night").unwrap();
        assert_eq!(state.engine.effect, Effect::Cycle { speed: 2.0 });
        assert_eq!(state.engine.brightness, 0.25);
        assert_eq!(state.to_save().active_profile.as_deref(), Some("Night"));
        assert_eq!(state.to_save().brightness, 0.25);
    }
}
