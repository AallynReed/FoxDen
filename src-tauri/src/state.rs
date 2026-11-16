use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use foxrgb_audio::AudioCapture;
use foxrgb_core::{
    Backend, DeviceKind, Effect, EffectInfo, Engine, Inputs, KeyInfo, Layer, Post, PostInfo,
    key_layout, presses_on,
};
use foxrgb_openrgb::OpenRgbBackend;
use foxrgb_screen::ScreenCapture;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::config::{self, Config, Profile, Settings};
use crate::detect::{self, Detected, Found, Live, Watch};
use crate::keys::KeyCapture;
use crate::tray;

const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Seconds after which effect time starts again from zero.
const CLOCK_WRAP: f64 = 21_600.0;
/// How long a backend gets to finish a push and let go of its device.
const RELEASE: Duration = Duration::from_secs(2);
/// How long the last black frame may take on the way out.
const LAST_FRAME: Duration = Duration::from_millis(500);
/// Backend groups in the order their devices are listed.
/// When, after start, an empty boards-and-memory group is probed again.
const BOARD_RETRIES: [Duration; 4] = [
    Duration::from_secs(3),
    Duration::from_secs(8),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

const GROUPS: [&str; 4] = ["openrgb", "hid", "board", "net"];

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
    /// Zones whose LED count the user sets.
    zones: Vec<ZoneView>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneView {
    key: String,
    name: String,
    leds: usize,
    min: usize,
    max: usize,
}

impl From<foxrgb_hid::Zone> for ZoneView {
    fn from(z: foxrgb_hid::Zone) -> Self {
        ZoneView {
            key: foxrgb_hid::length_key(&z.device, z.index),
            name: z.name,
            leds: z.leds,
            min: z.min,
            max: z.max,
        }
    }
}

#[derive(Serialize)]
pub struct Snapshot {
    devices: Vec<DeviceView>,
    effect: Effect,
    layers: Vec<Layer>,
    overrides: BTreeMap<String, Effect>,
    post_catalog: Vec<PostInfo>,
    key_layout: Vec<KeyInfo>,
    brightness: f32,
    pub paused: bool,
    catalog: Vec<EffectInfo>,
    pub profiles: Vec<String>,
    pub active_profile: Option<String>,
    settings: Settings,
    version: &'static str,
    backends: Vec<BackendStatus>,
    detected: Vec<Detected>,
}

/// A choice of backend groups to reconnect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kinds {
    pub openrgb: bool,
    pub hid: bool,
    /// Graphics cards, memory, Super I/O and WMI: probed at start and on demand, not on USB changes.
    pub board: bool,
    pub net: bool,
}

impl Kinds {
    pub const ALL: Kinds = Kinds {
        openrgb: true,
        hid: true,
        board: true,
        net: true,
    };

    pub fn any(self) -> bool {
        self.openrgb || self.hid || self.board || self.net
    }

    fn has(self, label: &str) -> bool {
        match label {
            "openrgb" => self.openrgb,
            "net" => self.net,
            "board" => self.board,
            _ => self.hid,
        }
    }
}

/// Work for the thread that connects backends and looks for hardware.
pub enum Job {
    /// Reconnect these backends, then look for hardware.
    Reload(Kinds),
    /// Look for hardware now.
    Detect,
}

pub type Jobs = Sender<Job>;

pub struct AppState {
    /// Owns the live effect and brightness; `config` holds everything else.
    pub engine: Engine,
    pub config: Config,
    /// When the unsaved changes were last touched.
    pub changed: Option<Instant>,
    /// Connection error, if any, of every backend group that is switched on.
    loaded: HashMap<&'static str, Option<String>>,
    inputs: Vec<BackendStatus>,
    retry_inputs: bool,
    /// Hardware the last detection pass found.
    found: Vec<Found>,
    /// Whether the OpenRGB server answered that pass.
    openrgb_up: bool,
    /// Resizable zones of the native devices, by device id.
    zones: HashMap<String, Vec<ZoneView>>,
}

pub type Shared = Arc<Mutex<AppState>>;

pub fn lock(shared: &Shared) -> MutexGuard<'_, AppState> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let mut engine = Engine::new(Vec::new());
        engine.effect = config.effect.clone();
        engine.layers = config.layers.clone();
        engine.overrides = config.overrides.clone().into_iter().collect();
        engine.brightness = config.brightness;
        for (id, device) in &config.devices {
            engine.set_device(id, device.enabled, device.brightness, device.rect);
        }
        // an empty group still takes its place in the order
        for label in GROUPS {
            engine.add(label, Vec::new());
        }
        AppState {
            engine,
            config,
            changed: None,
            loaded: HashMap::new(),
            inputs: Vec::new(),
            retry_inputs: false,
            found: Vec::new(),
            openrgb_up: false,
            zones: HashMap::new(),
        }
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
                zones: match backend {
                    "hid" => self.zones.get(&device.id).cloned().unwrap_or_default(),
                    _ => Vec::new(),
                },
            })
            .collect();
        Snapshot {
            devices,
            effect: self.engine.effect.clone(),
            layers: self.engine.layers.clone(),
            overrides: self.engine.overrides.clone().into_iter().collect(),
            post_catalog: Post::catalog(),
            key_layout: key_layout(),
            brightness: self.engine.brightness,
            paused: self.engine.paused,
            catalog: Effect::catalog(),
            profiles: self.config.profiles.keys().cloned().collect(),
            active_profile: self.config.active_profile.clone(),
            settings: self.config.settings.clone(),
            version: env!("CARGO_PKG_VERSION"),
            backends: self.backends(),
            detected: self.detected(),
        }
    }

    fn backends(&self) -> Vec<BackendStatus> {
        let slots = self.engine.status();
        let hardware = GROUPS.into_iter().map(|name| {
            let mine = || slots.iter().filter(|(label, ..)| label == name);
            let outcome = self.loaded.get(name);
            let error = outcome
                .cloned()
                .flatten()
                .or_else(|| mine().find_map(|(.., e)| e.clone()));
            BackendStatus {
                name: name.into(),
                active: outcome.is_some() && error.is_none(),
                devices: mine().map(|(_, count, _)| count).sum(),
                error,
            }
        });
        hardware.chain(self.inputs.iter().cloned()).collect()
    }

    /// Whether this backend group is switched on and working.
    fn connected(&self, name: &str) -> bool {
        self.backends().iter().any(|b| b.name == name && b.active)
    }

    /// Whether a backend of this group has stopped taking frames.
    fn failing(&self, name: &str) -> bool {
        self.engine
            .status()
            .iter()
            .any(|(label, _, error)| label == name && error.is_some())
    }

    /// Connected to OpenRGB and shown at least one device. An empty list is read again later,
    /// because a server that is still starting up announces its devices to nobody.
    fn openrgb_settled(&self) -> bool {
        self.backends()
            .iter()
            .any(|b| b.name == "openrgb" && b.active && b.devices > 0)
    }

    /// What detection found, joined with what the backends drive right now.
    fn detected(&self) -> Vec<Detected> {
        let live = Live {
            settings: &self.config.settings,
            driven: self.engine.driven(),
            openrgb_up: self.openrgb_up,
            openrgb_on: self.connected("openrgb"),
        };
        detect::resolve(&self.found, &live)
    }

    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        let profile = self
            .config
            .profiles
            .get(name)
            .ok_or_else(|| format!("no profile named {name}"))?;
        self.engine.effect = profile.effect.clone();
        self.engine.layers = profile.layers.clone();
        self.engine.overrides = profile.overrides.clone().into_iter().collect();
        self.engine.brightness = profile.brightness.clamp(0.0, 1.0);
        self.config.active_profile = Some(name.to_string());
        Ok(())
    }

    /// Forgets the active profile once the live effect or brightness no longer match it.
    pub fn check_profile(&mut self) {
        let live = Profile {
            effect: self.engine.effect.clone(),
            layers: self.engine.layers.clone(),
            overrides: self.engine.overrides.clone().into_iter().collect(),
            brightness: self.engine.brightness,
        };
        let name = self.config.active_profile.as_ref();
        if name.is_some_and(|n| self.config.profiles.get(n) != Some(&live)) {
            self.config.active_profile = None;
        }
    }

    fn to_save(&self) -> Config {
        Config {
            effect: self.engine.effect.clone(),
            layers: self.engine.layers.clone(),
            overrides: self.engine.overrides.clone().into_iter().collect(),
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
    note_changes(&snapshot);
    let _ = app.emit("snapshot", &snapshot);
    tray::refresh(app, &snapshot);
}

#[derive(Default)]
struct Seen {
    devices: HashMap<String, String>,
    errors: HashMap<String, Option<String>>,
    found: HashMap<String, detect::State>,
}

static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

/// Writes what changed since the last snapshot to the log.
fn note_changes(snapshot: &Snapshot) {
    let mut guard = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = guard.get_or_insert_with(Seen::default);
    let now: HashMap<String, String> = snapshot
        .devices
        .iter()
        .map(|d| (d.id.clone(), format!("{} ({})", d.name, d.backend)))
        .collect();
    for (id, name) in &now {
        if !seen.devices.contains_key(id) {
            crate::log::info("devices", format!("Connected {name}"));
        }
    }
    for (id, name) in &seen.devices {
        if !now.contains_key(id) {
            crate::log::info("devices", format!("Disconnected {name}"));
        }
    }
    seen.devices = now;
    for b in &snapshot.backends {
        let old = seen
            .errors
            .insert(b.name.clone(), b.error.clone())
            .flatten();
        match (&b.error, old) {
            (Some(e), old) if old.as_ref() != Some(e) => crate::log::error(&b.name, e.clone()),
            (None, Some(_)) => crate::log::info(&b.name, "Working again"),
            _ => {}
        }
    }
    for d in &snapshot.detected {
        if seen.found.insert(d.id.clone(), d.state) != Some(d.state)
            && d.state == detect::State::Unsupported
        {
            crate::log::warn("devices", format!("{}: {}", d.name, d.note));
        }
    }
    crate::report::set_devices(snapshot.detected.iter().map(|d| {
        let driver = match d.route {
            detect::Route::Native => "native",
            detect::Route::Openrgb => "openrgb",
            detect::Route::None => "none",
        };
        (d.id.clone(), driver.to_string())
    }));
}

/// Starts the one thread that connects backends and looks for hardware, so neither holds up a command.
pub fn spawn_worker(app: AppHandle, shared: Shared) -> Jobs {
    let (jobs, inbox) = mpsc::channel();
    thread::spawn(move || {
        let mut watch = Watch::default();
        let first = lock(&shared).config.settings.clone();
        // Closed before the devices are opened, so they are free to take.
        if first.close_conflicts {
            close_conflicts(&app, &first);
        }
        let enabled = Kinds {
            openrgb: enabled("openrgb", &first),
            hid: enabled("hid", &first),
            board: enabled("board", &first),
            net: enabled("net", &first),
        };
        if enabled.any() {
            reload(&shared, &mut watch, enabled);
            emit_snapshot(&app, &shared);
        }
        let mut system = Vec::new();
        let mut job = Some(Job::Detect);
        let started = Instant::now();
        let mut board_tries = 0;
        loop {
            let asked = job.is_some();
            let detect_asked = matches!(job, Some(Job::Detect));
            let mut done = Kinds::default();
            if let Some(Job::Reload(kinds)) = job {
                reload(&shared, &mut watch, kinds);
                emit_snapshot(&app, &shared);
                done = kinds;
            }
            let (settings, settled, failing) = {
                let state = lock(&shared);
                (
                    state.config.settings.clone(),
                    state.openrgb_settled(),
                    state.failing("hid"),
                )
            };
            if settings.close_conflicts && (asked || settings.auto_detect) {
                close_conflicts(&app, &settings);
            }
            if settings.lighting_module && (asked || settings.auto_detect) {
                // The board, memory and cards stay put: WMI is asked again only while it has given nothing.
                if asked && system.is_empty() {
                    system = detect::system();
                }
                let (found, print) = detect::scan(&system, &settings, settled);
                let openrgb_up = print.openrgb;
                let change = watch.step(print, &settings, settled);
                let died = watch.died(failing);
                {
                    let mut state = lock(&shared);
                    state.found = found;
                    state.openrgb_up = openrgb_up;
                }
                // Only what the user has switched on and whose hardware changed is connected again.
                let hid = (change.native || died) && !done.hid;
                let stale = Kinds {
                    openrgb: change.openrgb && !done.openrgb,
                    hid,
                    board: false,
                    net: false,
                };
                if stale.any() {
                    reload(&shared, &mut watch, stale);
                }
                // At start the service may answer before its memory driver is ready: look again a few times.
                let none_yet = !lock(&shared)
                    .engine
                    .views()
                    .any(|(label, ..)| label == "board");
                if none_yet
                    && self::enabled("board", &settings)
                    && BOARD_RETRIES
                        .get(board_tries)
                        .is_some_and(|&at| started.elapsed() >= at)
                {
                    board_tries += 1;
                    let board = Kinds {
                        board: true,
                        ..Kinds::default()
                    };
                    reload(&shared, &mut watch, board);
                    emit_snapshot(&app, &shared);
                }
                if detect_asked || change.seen || stale.any() {
                    emit_snapshot(&app, &shared);
                }
            }
            job = match inbox.recv_timeout(detect::POLL) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
        }
    });
    jobs
}

fn enabled(label: &str, settings: &Settings) -> bool {
    settings.lighting_module
        && match label {
            "openrgb" => settings.openrgb_enabled,
            "net" => settings.network_enabled,
            _ => settings.native_drivers,
        }
}

/// Closes conflicting software and tells the UI, and with the window hidden a toast too.
fn close_conflicts(app: &AppHandle, settings: &Settings) {
    let report = crate::conflicts::sweep(crate::conflicts::scope(settings), &settings.keep_running);
    if !crate::conflicts::remember(report.clone()) {
        return;
    }
    let Some(report) = report else { return };
    crate::log::info("conflicts", report.text.clone());
    let _ = app.emit("conflicts", &report);
    let hidden = app
        .get_webview_window("main")
        .is_none_or(|w| !w.is_visible().unwrap_or(false));
    if hidden {
        let _ = app
            .notification()
            .builder()
            .title("FoxRGB")
            .body(&report.text)
            .show();
    }
}

/// Connects one group and logs how long it took.
fn timed(
    label: &str,
    connect: impl FnOnce() -> (Vec<Box<dyn Backend>>, Option<String>),
) -> (Vec<Box<dyn Backend>>, Option<String>) {
    let start = std::time::Instant::now();
    let out = connect();
    let devices: usize = out.0.iter().map(|b| b.devices().len()).sum();
    crate::log::info(
        "devices",
        format!(
            "{label}: {devices} devices in {} ms",
            start.elapsed().as_millis()
        ),
    );
    out
}

/// The backends of one group and the reason there are none, if it failed.
fn connect(label: &str, settings: &Settings) -> (Vec<Box<dyn Backend>>, Option<String>) {
    match label {
        "openrgb" => match OpenRgbBackend::connect(&settings.openrgb_host, settings.openrgb_port) {
            Ok(backend) => (vec![Box::new(backend)], None),
            Err(e) => (Vec::new(), Some(e)),
        },
        "net" => {
            let mut backends = foxrgb_net::probe(&settings.network);
            backends.extend(foxrgb_xpack::probe_network(&settings.network));
            (backends, None)
        }
        "board" => {
            let mut backends = foxrgb_gpu::probe();
            // Memory needs PawnIO and administrator rights, or the installed service that has them.
            match foxrgb_smbus::probe() {
                Ok(more) => {
                    backends.extend(more);
                    backends.extend(foxrgb_superio::lights());
                    backends.extend(foxrgb_wmi::lights());
                }
                Err(_) => {
                    if let Some(service) = crate::service_lights::probe() {
                        backends.push(Box::new(service));
                    }
                }
            }
            (backends, None)
        }
        _ => {
            // Strips first, so the serial probes never write to their ports.
            let strips: Vec<_> = settings
                .led_strips
                .iter()
                .filter_map(|s| s.strip())
                .collect();
            let step = |what: &str, start: std::time::Instant| {
                crate::log::info(
                    "devices",
                    format!("{what}: {} ms", start.elapsed().as_millis()),
                )
            };
            let start = std::time::Instant::now();
            let mut backends = foxrgb_hid::led_strips(&strips);
            step("LED strips", start);
            // Hubs whose fans the service drives get their lighting from it too, admin or not.
            let start = std::time::Instant::now();
            let hubs = crate::service_lights::hubs(&settings.zone_leds);
            step("fan hub lighting from the service", start);
            let skip = crate::service_lights::left_to_service(hubs.is_some());
            foxrgb_hid::set_lengths(settings.zone_leds.clone());
            let pick = foxrgb_hid::Pick::Skip(&skip);
            let start = std::time::Instant::now();
            backends.extend(foxrgb_hid::probe_picked(&foxrgb_xpack::detectors(), pick));
            step("USB HID devices", start);
            let start = std::time::Instant::now();
            backends.extend(foxrgb_xpack::probe_other_picked(pick));
            step("other USB devices", start);
            if let Some(hubs) = hubs {
                backends.push(Box::new(hubs));
            }
            for (what, ms) in foxrgb_hid::slow_opens() {
                crate::log::info("devices", format!("{what} took {ms} ms to open"));
            }
            (backends, None)
        }
    }
}

fn group_zones(zones: Vec<foxrgb_hid::Zone>) -> HashMap<String, Vec<ZoneView>> {
    let mut by_device: HashMap<String, Vec<ZoneView>> = HashMap::new();
    for z in zones {
        by_device
            .entry(z.device.clone())
            .or_default()
            .push(z.into());
    }
    by_device
}

/// Reconnects the chosen backend groups and leaves the others running. Worker thread only.
fn reload(shared: &Shared, watch: &mut Watch, kinds: Kinds) {
    let (settings, old) = {
        let mut state = lock(shared);
        state.retry_inputs = true;
        let mut old = Vec::new();
        if !state.config.settings.lighting_module {
            // Like quitting: one black frame, then every device is let go.
            state.engine.blackout();
            state.engine.flush(LAST_FRAME);
            old.push(state.engine.remove("openrgb"));
            old.push(state.engine.remove("hid"));
            old.push(state.engine.remove("board"));
            old.push(state.engine.remove("net"));
        } else {
            // HID devices and DMX ports have to be let go before they are probed again.
            if kinds.hid {
                old.push(state.engine.remove("hid"));
            }
            if kinds.board {
                old.push(state.engine.remove("board"));
            }
            if kinds.net {
                old.push(state.engine.remove("net"));
            }
        }
        (state.config.settings.clone(), old)
    };
    for old in old {
        old.wait(RELEASE);
    }
    let mut fresh = Vec::new();
    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let jobs: Vec<_> = GROUPS
            .into_iter()
            .filter(|label| kinds.has(label))
            .map(|label| {
                let settings = &settings;
                (
                    label,
                    scope.spawn(move || {
                        enabled(label, settings).then(|| timed(label, || connect(label, settings)))
                    }),
                )
            })
            .collect();
        jobs.into_iter()
            .map(|(label, job)| (label, job.join().ok().flatten()))
            .collect()
    });
    for (label, outcome) in outcomes {
        if label == "openrgb"
            && let Some((backends, _)) = &outcome
        {
            watch.tried(backends.iter().any(|backend| !backend.devices().is_empty()));
        }
        fresh.push((label, outcome));
    }
    let zones = kinds.hid.then(|| {
        let mut zones = foxrgb_hid::resizable();
        zones.extend(crate::service_lights::hub_zones());
        zones
    });
    let mut state = lock(shared);
    if let Some(zones) = zones {
        state.zones = group_zones(zones);
    }
    for (label, outcome) in fresh {
        state.engine.remove(label);
        state.loaded.remove(label);
        // If the switch was flipped while we were connecting, the job that queued settles it.
        if let Some((backends, error)) = outcome
            && enabled(label, &state.config.settings)
        {
            state.engine.add(label, backends);
            state.loaded.insert(label, error);
        }
    }
}

/// An audio or screen capture that only runs while the effect needs it.
struct Input<T> {
    capture: Option<T>,
    error: Option<String>,
}

impl<T> Input<T> {
    fn off() -> Self {
        Input {
            capture: None,
            error: None,
        }
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
            Ok(capture) => {
                *self = Input {
                    capture: Some(capture),
                    error: None,
                }
            }
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
        let mut keys = Input::<KeyCapture>::off();
        loop {
            let (fps, want_audio, want_screen, want_keys, retry) = {
                let mut state = lock(&shared);
                let on = state.config.settings.lighting_module;
                let (audio, screen, keys) = state.engine.wants();
                (
                    state.config.settings.fps,
                    on && audio,
                    on && screen,
                    on && keys,
                    std::mem::take(&mut state.retry_inputs),
                )
            };
            let audio_changed = audio.sync(want_audio, retry, AudioCapture::start);
            let screen_changed = screen.sync(want_screen, retry, ScreenCapture::start);
            let keys_changed = keys.sync(want_keys, retry, KeyCapture::start);
            let pressed = keys.capture.as_ref().map(|c| c.frame()).unwrap_or_default();
            inputs.audio = audio
                .capture
                .as_ref()
                .map(|c| c.frame())
                .unwrap_or_default();
            inputs.screen = screen
                .capture
                .as_ref()
                .map(|c| c.frame())
                .unwrap_or_default();

            let changed = audio_changed || screen_changed || keys_changed;
            // The engine only queues the frame here; each backend sends it on its own thread.
            let (frames, failed) = {
                let mut state = lock(&shared);
                if changed {
                    state.inputs = [
                        audio.status("audio"),
                        screen.status("screen"),
                        keys.status("keys"),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                }
                // f32 seconds turn coarse after days of uptime, so the clock wraps.
                let t = (start.elapsed().as_secs_f64() % CLOCK_WRAP) as f32;
                inputs.keyboards = state
                    .engine
                    .views()
                    .filter(|(_, d, on, _)| *on && d.kind == DeviceKind::Keyboard)
                    .map(|(_, d, _, _)| (d.rect, d.grid[0] >= 21))
                    .collect();
                inputs.presses = presses_on(&pressed, &inputs.keyboards);
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
        state.engine.flush(LAST_FRAME);
        state.changed.take().map(|_| state.to_save())
    };
    if let Some(config) = config {
        save(&config);
    }
}

fn save(config: &Config) {
    if let Err(e) = config::save(&config::dir(), config) {
        crate::log::error("config", format!("settings not saved: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Bus;
    use foxrgb_core::{Color, Device, VirtualBackend};
    use serde_json::{Value, json};
    use std::net::TcpListener;

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    /// The defaults, with test devices in the native group.
    fn with_devices() -> AppState {
        let mut state = AppState::new(Config::default());
        state.engine.add("hid", vec![Box::new(VirtualBackend)]);
        state.loaded.insert("hid", None);
        state
    }

    #[test]
    fn snapshot_has_the_shape_the_ui_expects() {
        let mut state = with_devices();
        state
            .engine
            .set_device("virtual:gpu", false, 0.5, Some([0.25, 0.5, 0.25, 0.125]));
        let json = serde_json::to_value(state.snapshot()).unwrap();
        assert_eq!(
            keys(&json),
            [
                "active_profile",
                "backends",
                "brightness",
                "catalog",
                "detected",
                "devices",
                "effect",
                "key_layout",
                "layers",
                "overrides",
                "paused",
                "post_catalog",
                "profiles",
                "settings",
                "version"
            ]
        );
        assert_eq!(
            keys(&json["settings"]),
            [
                "auto_detect",
                "auto_update",
                "autostart",
                "close_conflicts",
                "close_to_tray",
                "fans_module",
                "fps",
                "keep_running",
                "led_strips",
                "lighting_module",
                "native_drivers",
                "network",
                "network_enabled",
                "openrgb_enabled",
                "openrgb_host",
                "openrgb_port",
                "send_reports",
                "start_minimized",
                "zone_leds"
            ]
        );
        assert_eq!(json["settings"]["send_reports"], false);
        assert_eq!(json["settings"]["native_drivers"], true);
        assert_eq!(json["settings"]["autostart"], true);
        assert_eq!(json["settings"]["auto_detect"], true);
        assert_eq!(json["settings"]["network_enabled"], false);
        assert_eq!(json["settings"]["network"]["hue"], json!([]));
        assert_eq!(json["detected"], json!([]));

        let gpu = json["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "virtual:gpu")
            .unwrap();
        assert_eq!(
            keys(gpu),
            [
                "backend",
                "brightness",
                "enabled",
                "grid",
                "id",
                "kind",
                "leds",
                "name",
                "rect",
                "zones"
            ]
        );
        assert_eq!(gpu["zones"], json!([]));
        state.zones = group_zones(vec![foxrgb_hid::Zone {
            device: "virtual:gpu".into(),
            index: 1,
            name: "Header 1".into(),
            leds: 30,
            min: 0,
            max: 120,
        }]);
        let json = serde_json::to_value(state.snapshot()).unwrap();
        let gpu = json["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "virtual:gpu")
            .unwrap();
        assert_eq!(
            gpu["zones"],
            json!([{ "key": "virtual:gpu#1", "name": "Header 1", "leds": 30, "min": 0, "max": 120 }])
        );
        assert_eq!(gpu["backend"], "hid");
        assert_eq!(gpu["kind"], "gpu");
        assert_eq!(gpu["enabled"], false);
        assert_eq!(gpu["brightness"], 0.5);
        assert_eq!(gpu["rect"], json!([0.25, 0.5, 0.25, 0.125]));
        assert_eq!(gpu["grid"], json!([12, 1]));

        let backends = json["backends"].as_array().unwrap();
        assert_eq!(
            backends[0],
            json!({ "name": "openrgb", "active": false, "devices": 0, "error": null })
        );
        assert_eq!(
            backends[1],
            json!({ "name": "hid", "active": true, "devices": 6, "error": null })
        );
        assert_eq!(
            backends[2],
            json!({ "name": "board", "active": false, "devices": 0, "error": null })
        );
        assert_eq!(
            backends[3],
            json!({ "name": "net", "active": false, "devices": 0, "error": null })
        );
        assert_eq!(backends.len(), 4);
    }

    #[test]
    fn snapshot_lists_what_detection_found() {
        let mut state = AppState::new(Config::default());
        state.found = vec![Found {
            id: "usb:1b1c:0001".into(),
            name: "Corsair device".into(),
            kind: "keyboard",
            bus: Bus::Usb,
            native: None,
            brand: Some("Corsair"),
        }];
        state.openrgb_up = true;
        let json = serde_json::to_value(state.snapshot()).unwrap();
        let detected = json["detected"].as_array().unwrap();
        assert_eq!(detected.len(), 2);
        assert_eq!(
            keys(&detected[0]),
            ["bus", "id", "kind", "name", "note", "route", "state"]
        );
        assert_eq!(detected[0]["id"], "usb:1b1c:0001");
        assert_eq!(detected[0]["route"], "openrgb");
        assert_eq!(detected[0]["state"], "available");
        assert_eq!(detected[1]["id"], "network:openrgb");
        assert_eq!(detected[1]["state"], "available");
    }

    #[test]
    fn a_native_backend_that_stops_taking_frames_is_noticed() {
        struct Unplugged;
        impl Backend for Unplugged {
            fn name(&self) -> &str {
                "unplugged"
            }

            fn devices(&self) -> Vec<Device> {
                vec![Device::grid(
                    "hid:unplugged",
                    "Unplugged",
                    DeviceKind::Mouse,
                    [0.0; 4],
                    1,
                    1,
                )]
            }

            fn push(&mut self, _device_id: &str, _colors: &[Color]) -> Result<(), String> {
                Err("the device is not connected".into())
            }
        }

        let mut state = AppState::new(Config::default());
        state.engine.add("hid", vec![Box::new(Unplugged)]);
        assert!(!state.failing("hid"));
        state.engine.render(0.0, &Inputs::default());
        state.engine.flush(Duration::from_secs(5));
        assert!(state.failing("hid"));
        assert!(!state.failing("openrgb"));
    }

    #[test]
    fn reload_swaps_one_group_and_leaves_the_rest() {
        let shared: Shared = Arc::new(Mutex::new(with_devices()));
        let mut watch = Watch::default();
        let status = |name: &str| {
            lock(&shared)
                .backends()
                .into_iter()
                .find(|b| b.name == name)
                .unwrap()
        };
        let free = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        {
            let mut state = lock(&shared);
            state.config.settings.openrgb_enabled = true;
            state.config.settings.openrgb_port = free;
        }
        reload(
            &shared,
            &mut watch,
            Kinds {
                openrgb: true,
                ..Kinds::default()
            },
        );
        assert!(status("openrgb").error.is_some());
        assert!(!lock(&shared).connected("openrgb"));
        assert_eq!((status("hid").active, status("hid").devices), (true, 6));

        lock(&shared).config.settings.openrgb_enabled = false;
        reload(
            &shared,
            &mut watch,
            Kinds {
                openrgb: true,
                ..Kinds::default()
            },
        );
        assert_eq!(status("hid").devices, 6);
        assert_eq!(
            (status("openrgb").active, status("openrgb").error),
            (false, None)
        );
        let first = lock(&shared)
            .snapshot()
            .devices
            .first()
            .map(|d| d.id.clone());
        assert_eq!(first.as_deref(), Some("virtual:board"));
    }

    #[test]
    fn switching_lighting_off_lets_every_device_go() {
        let shared: Shared = Arc::new(Mutex::new(with_devices()));
        lock(&shared).config.settings.lighting_module = false;
        reload(&shared, &mut Watch::default(), Kinds::ALL);
        let state = lock(&shared);
        assert!(state.snapshot().devices.is_empty());
        assert!(
            state
                .backends()
                .iter()
                .all(|b| !b.active && b.error.is_none())
        );
    }

    #[test]
    fn loading_a_profile_replaces_effect_and_brightness() {
        let mut state = AppState::new(Config::default());
        let profile = config::Profile {
            effect: Effect::Cycle { speed: 2.0 },
            layers: Vec::new(),
            overrides: BTreeMap::new(),
            brightness: 0.25,
        };
        state.config.profiles.insert("Night".into(), profile);
        assert!(state.load_profile("Day").is_err());
        state.load_profile("Night").unwrap();
        assert_eq!(state.engine.effect, Effect::Cycle { speed: 2.0 });
        assert_eq!(state.engine.brightness, 0.25);
        assert_eq!(state.to_save().active_profile.as_deref(), Some("Night"));
        assert_eq!(state.to_save().brightness, 0.25);
    }

    #[test]
    fn an_edited_profile_is_no_longer_active() {
        let mut state = AppState::new(Config::default());
        let profile = Profile {
            effect: Effect::Cycle { speed: 2.0 },
            layers: Vec::new(),
            overrides: BTreeMap::new(),
            brightness: 0.25,
        };
        state.config.profiles.insert("Night".into(), profile);
        state.load_profile("Night").unwrap();
        state.check_profile();
        assert_eq!(state.config.active_profile.as_deref(), Some("Night"));
        state.engine.brightness = 0.5;
        state.check_profile();
        assert_eq!(state.config.active_profile, None);
    }
}
