//! The mixer: channels fed by devices and by single apps, with volume, chat mix and EQ.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use foxden_mixer::Channel;
use foxden_mixer::dsp::{self, Band, FilterType};
use foxden_mixer::park::{self, AppState, Parker, Plan, RunningApp};
use foxden_mixer::router::{
    Controls, DeviceList, DeviceRoute, MixState, Router, VirtualMic, list_devices,
};
use foxden_mixer::system;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::profiles::{self, Gate, Kind, Profile};
use crate::state::{Shared, lock};

const CURVE_POINTS: usize = 96;
const LAUNCH_TRIES: u32 = 6;
const LAUNCH_WAIT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_secs(1);
const JOURNAL: &str = "audio-parked.json";
const VMIC_RETRY: Duration = Duration::from_secs(5);
const GATE_DB: f32 = -50.0;
const FOLLOW_RETRY: Duration = Duration::from_secs(10);
const AUTO_EVERY: Duration = Duration::from_secs(2);
const STOP_WAIT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    pub channel: Channel,
    pub source: String,
    /// The source is an output: what plays on it is recorded.
    pub loopback: bool,
    /// Executables whose sound goes to this channel.
    pub apps: Vec<String>,
    pub on: bool,
    pub volume: f32,
    pub muted: bool,
    pub eq_on: bool,
    pub eq: Vec<Band>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self::new(Channel::Game)
    }
}

impl ChannelConfig {
    fn new(channel: Channel) -> Self {
        ChannelConfig {
            channel,
            source: String::new(),
            loopback: !channel.is_capture(),
            apps: Vec::new(),
            on: false,
            volume: 1.0,
            muted: false,
            eq_on: false,
            eq: dsp::default_bands(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub output: String,
    /// Output assigned apps are parked on, muted; empty picks one.
    pub park: String,
    pub channels: Vec<ChannelConfig>,
    pub master: f32,
    pub master_muted: bool,
    pub chat_mix: f32,
    /// Running when FoxDen last closed, so it starts again with FoxDen.
    pub running: bool,
    /// The mic is played into VB-CABLE for apps to record.
    pub virtual_mic: bool,
    pub gate_on: bool,
    pub gate_db: f32,
    pub priority: crate::priority::PriorityConfig,
    /// Apps that start playing go to their preset channel.
    pub auto_assign: bool,
    /// Lowercase executables placed by auto-assign, not by hand.
    pub auto: Vec<String>,
    /// Lowercase executables auto-assign leaves alone.
    pub no_auto: Vec<String>,
    /// Saved and imported sound profiles; the built-in ones are not stored.
    pub profiles: Vec<Profile>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            output: String::new(),
            park: String::new(),
            channels: Channel::ALL.map(ChannelConfig::new).to_vec(),
            master: 1.0,
            master_muted: false,
            chat_mix: 0.0,
            running: false,
            virtual_mic: false,
            gate_on: false,
            gate_db: GATE_DB,
            priority: Default::default(),
            auto_assign: true,
            auto: Vec::new(),
            no_auto: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

/// What the audio thread plays.
#[derive(Clone, Debug, PartialEq)]
struct Spec {
    output: String,
    routes: Vec<DeviceRoute>,
    park: String,
    apps: BTreeMap<String, Channel>,
    /// Outputs never to park on: the output priority list, by name and id.
    avoid: Vec<String>,
}

impl AudioConfig {
    /// One entry per channel, values in range, each app on one channel and never on the mic.
    fn tidy(mut self) -> Self {
        let mut seen: Vec<String> = Vec::new();
        self.channels = Channel::ALL
            .map(|c| {
                let mut ch = self
                    .channels
                    .iter()
                    .find(|x| x.channel == c)
                    .cloned()
                    .unwrap_or_else(|| ChannelConfig::new(c));
                ch.volume = finite(ch.volume, 1.0).clamp(0.0, 1.0);
                ch.eq.truncate(dsp::MAX_BANDS);
                if ch.eq.is_empty() {
                    ch.eq = dsp::default_bands();
                }
                if c.is_capture() {
                    ch.apps.clear();
                }
                ch.apps.retain(|a| {
                    let k = park::lower(a);
                    !a.trim().is_empty() && !seen.contains(&k) && {
                        seen.push(k);
                        true
                    }
                });
                ch
            })
            .to_vec();
        self.master = finite(self.master, 1.0).clamp(0.0, 1.0);
        self.chat_mix = finite(self.chat_mix, 0.0).clamp(-1.0, 1.0);
        self.gate_db = finite(self.gate_db, GATE_DB).clamp(-90.0, 0.0);
        self.priority = std::mem::take(&mut self.priority).tidy();
        let placed = self.apps_anywhere();
        for list in [&mut self.auto, &mut self.no_auto] {
            for a in list.iter_mut() {
                *a = park::lower(a);
            }
            list.sort();
            list.dedup();
        }
        self.auto.retain(|a| placed.contains(a));
        self
    }

    /// Every assigned app, lowercase, on or off.
    fn apps_anywhere(&self) -> Vec<String> {
        self.channels
            .iter()
            .flat_map(|c| c.apps.iter().map(|a| park::lower(a)))
            .collect()
    }

    /// The mic source the virtual mic plays, while it is on.
    fn vmic(&self) -> Option<(String, bool)> {
        let mic = self.channels.iter().find(|c| c.channel.is_capture())?;
        (self.virtual_mic && !mic.source.is_empty()).then(|| (mic.source.clone(), mic.loopback))
    }

    fn channel(&mut self, c: Channel) -> &mut ChannelConfig {
        let i = self
            .channels
            .iter()
            .position(|x| x.channel == c)
            .unwrap_or(0);
        &mut self.channels[i]
    }

    fn apps(&self) -> BTreeMap<String, Channel> {
        self.channels
            .iter()
            .filter(|c| c.on && !c.channel.is_capture())
            .flat_map(|c| c.apps.iter().map(|a| (park::lower(a), c.channel)))
            .collect()
    }

    /// What to play, refusing any route that would feed an output back into itself.
    fn spec(&self) -> Result<Spec, String> {
        if self.output.is_empty() {
            return Err("Pick an output to play on".into());
        }
        let routes: Vec<DeviceRoute> = self
            .channels
            .iter()
            .filter(|c| c.on && !c.source.is_empty())
            .map(|c| DeviceRoute {
                channel: c.channel,
                source: c.source.clone(),
                source_is_output: c.loopback,
            })
            .collect();
        if let Some(r) = routes
            .iter()
            .find(|r| r.source_is_output && r.source == self.output)
        {
            return Err(format!(
                "{} would play back into itself; pick another source or output",
                r.channel.label()
            ));
        }
        let apps = self.apps();
        if routes.is_empty() && apps.is_empty() {
            return Err("Turn on at least one channel with a source or an app".into());
        }
        Ok(Spec {
            output: self.output.clone(),
            routes,
            park: self.park.clone(),
            apps,
            avoid: self
                .priority
                .outputs
                .iter()
                .flat_map(|e| [e.id.clone(), e.name.clone()])
                .collect(),
        })
    }

    fn state(&self) -> MixState {
        let mut s = MixState::default();
        (s.master, s.master_muted, s.chat_mix) = (self.master, self.master_muted, self.chat_mix);
        for c in &self.channels {
            s.set_volume(c.channel, c.volume);
            s.set_muted(c.channel, c.muted);
            s.set_eq(c.channel, c.eq_on, c.eq.clone());
        }
        s.gate = self.gate();
        s
    }

    fn gate(&self) -> Option<f32> {
        self.gate_on.then(|| dsp::db_to_linear(self.gate_db))
    }
}

fn finite(v: f32, or: f32) -> f32 {
    if v.is_finite() { v } else { or }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Output {
        name: String,
    },
    Park {
        name: String,
    },
    Route {
        channel: Channel,
        source: String,
        loopback: bool,
        on: bool,
    },
    Apps {
        channel: Channel,
        apps: Vec<String>,
    },
    Volume {
        channel: Channel,
        volume: f32,
    },
    Mute {
        channel: Channel,
        muted: bool,
    },
    Master {
        volume: f32,
    },
    MasterMute {
        muted: bool,
    },
    ChatMix {
        balance: f32,
    },
    Eq {
        channel: Channel,
        enabled: bool,
        bands: Vec<Band>,
    },
    Run {
        on: bool,
    },
    VirtualMic {
        on: bool,
    },
    Gate {
        on: bool,
        threshold_db: f32,
    },
    AutoAssign {
        on: bool,
    },
    /// Lets auto-assign place this app again.
    AllowAuto {
        exe: String,
    },
    ApplyProfile {
        channel: Channel,
        profile: Profile,
    },
    SaveProfile {
        channel: Channel,
        name: String,
    },
    AddProfile {
        profile: Profile,
    },
    RenameProfile {
        from: String,
        to: String,
    },
    DeleteProfile {
        name: String,
    },
}

fn kind_of(c: Channel) -> Kind {
    if c.is_capture() {
        Kind::Mic
    } else {
        Kind::Output
    }
}

/// Adds or replaces a stored profile by name.
fn store(list: &mut Vec<Profile>, p: Profile) -> Result<(), String> {
    if profiles::builtins().iter().any(|b| b.name == p.name) {
        return Err(format!(
            "{} is a built-in profile; pick another name",
            p.name
        ));
    }
    let full = list.len() >= profiles::MAX_PROFILES;
    match list.iter_mut().find(|x| x.name == p.name) {
        Some(x) => *x = p,
        None if full => return Err("Too many profiles; delete one first".into()),
        None => list.push(p),
    }
    Ok(())
}

/// What the audio thread last saw of the apps.
#[derive(Clone, Debug, Default)]
struct Parked {
    states: BTreeMap<String, AppState>,
    sink: Option<String>,
    error: Option<String>,
}

/// The virtual mic as the panel shows it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct VmicView {
    /// VB-CABLE's driver is present.
    installed: bool,
    /// FoxDen installed it, so FoxDen may remove it.
    ours: bool,
    /// Both endpoints are up; until then Windows may need a restart.
    ready: bool,
    /// The recording endpoint's name, as apps list it.
    name: Option<String>,
    playing: bool,
    error: Option<String>,
}

/// What the audio thread keeps for the virtual mic.
#[derive(Default)]
struct Vmic {
    want: Option<(String, bool)>,
    playing: Option<((String, bool, String), VirtualMic)>,
    failed: Option<((String, bool, String), Instant)>,
}

impl Vmic {
    /// Starts, restarts or stops the virtual mic to match `want` and VB-CABLE's feed endpoint.
    fn tick(&mut self, controls: &Controls, view: &Mutex<VmicView>) {
        let status = foxden_vcable::status();
        let feed = status.feed.as_ref().filter(|e| e.active);
        let key = self
            .want
            .clone()
            .zip(feed)
            .map(|((s, l), f)| (s, l, f.name.clone()));
        let mut error = None;
        match key {
            None => self.playing = None,
            Some(k) if self.playing.as_ref().is_some_and(|(p, _)| *p == k) => {}
            Some(k) => {
                self.playing = None;
                let wait = self
                    .failed
                    .as_ref()
                    .is_some_and(|(f, at)| *f == k && at.elapsed() < VMIC_RETRY);
                if !wait {
                    match VirtualMic::start(&k.0, k.1, &k.2, controls.clone()) {
                        Ok(v) => {
                            self.failed = None;
                            self.playing = Some((k, v));
                        }
                        Err(e) => {
                            crate::log::warn("audio", format!("virtual mic did not start: {e}"));
                            self.failed = Some((k, Instant::now()));
                        }
                    }
                }
                if self.playing.is_none() {
                    error = Some("The virtual mic could not start; check the microphone".into());
                }
            }
        }
        *view.lock().unwrap_or_else(|e| e.into_inner()) = VmicView {
            installed: status.driver,
            ours: status.ours,
            ready: status.ready(),
            name: status.mic.map(|e| e.name),
            playing: self.playing.is_some(),
            error,
        };
    }
}

/// VB-CABLE's endpoints are never offered as sources, outputs or a place to park apps.
fn hidden() -> Vec<foxden_vcable::Endpoint> {
    foxden_vcable::endpoints().unwrap_or_default()
}

enum Job {
    Vmic(Option<(String, bool)>),
    Devices(SyncSender<Result<DeviceList, String>>),
    Apps(SyncSender<Vec<RunningApp>>),
    /// Executable name and path of each app playing sound now.
    Sounding(SyncSender<Vec<(String, String)>>),
    Start(Spec, SyncSender<Result<(), String>>),
    /// New app assignments or parking output while playing.
    Plan(Spec),
    /// Answers once the apps are put back.
    Stop(Option<SyncSender<()>>),
    Shutdown(SyncSender<()>),
}

struct Inner {
    config: AudioConfig,
    running: bool,
    error: Option<String>,
    devices: Option<DeviceList>,
    apps: Vec<RunningApp>,
    /// The output the priority picked, in place of the configured one.
    follow: Option<String>,
    playing_on: Option<String>,
    tried: Option<Instant>,
}

impl Inner {
    fn output(&self) -> &str {
        self.follow.as_deref().unwrap_or(&self.config.output)
    }

    fn spec(&self) -> Result<Spec, String> {
        match &self.follow {
            None => self.config.spec(),
            Some(o) => AudioConfig {
                output: o.clone(),
                ..self.config.clone()
            }
            .spec(),
        }
    }
}

#[derive(Clone)]
pub struct Mixer {
    inner: Arc<Mutex<Inner>>,
    controls: Controls,
    parked: Arc<Mutex<Parked>>,
    vmic: Arc<Mutex<VmicView>>,
    jobs: Arc<Mutex<Sender<Job>>>,
    shared: Shared,
}

struct Playing {
    router: Router,
    spec: Spec,
}

/// Parks the spec's apps and starts or stops their captures. Runs on the audio thread.
fn park_apps(parker: &mut Parker, playing: Option<&mut Playing>, parked: &Mutex<Parked>) {
    let mut out = Parked::default();
    match playing {
        None => {
            parker.tick(None);
        }
        Some(p) if p.spec.apps.is_empty() => {
            parker.tick(None);
            p.router.set_apps(&[]);
        }
        Some(p) => {
            let hide = hidden();
            let mut endpoints = system::endpoints().unwrap_or_default();
            endpoints.retain(|e| !hide.iter().any(|h| h.id == e.id));
            let avoid: Vec<&str> = p
                .spec
                .routes
                .iter()
                .filter(|r| r.source_is_output)
                .map(|r| r.source.as_str())
                .chain(p.spec.avoid.iter().map(String::as_str))
                .collect();
            match park::choose_sink(&endpoints, &p.spec.park, &p.spec.output, &avoid) {
                Ok(sink) => {
                    out.sink = Some(sink.name.clone());
                    let plan = Plan {
                        sink,
                        apps: p.spec.apps.clone(),
                    };
                    let (caps, mut states) = parker.tick(Some(&plan));
                    p.router.set_apps(&caps);
                    let exe: BTreeMap<u32, String> = system::processes()
                        .into_iter()
                        .map(|x| (x.pid, park::lower(&x.exe)))
                        .collect();
                    for (pid, message) in p.router.app_errors() {
                        if let Some(name) = exe.get(&pid) {
                            states.insert(name.clone(), AppState::Error { message });
                        }
                    }
                    out.states = states;
                }
                Err(e) => {
                    parker.tick(None);
                    p.router.set_apps(&[]);
                    out.error = Some(e);
                }
            }
        }
    }
    *parked.lock().unwrap_or_else(|e| e.into_inner()) = out;
}

fn sounding() -> Vec<(String, String)> {
    let procs = system::processes();
    let own = std::process::id();
    let mut out: Vec<(String, String)> = Vec::new();
    for s in system::sessions().unwrap_or_default() {
        if !s.active || s.pid == own {
            continue;
        }
        let Some(p) = procs.iter().find(|p| p.pid == s.pid) else {
            continue;
        };
        if !out.iter().any(|(e, _)| e.eq_ignore_ascii_case(&p.exe)) {
            out.push((p.exe.clone(), system::exe_path(s.pid).unwrap_or_default()));
        }
    }
    out
}

/// Puts back parked apps and the parking output's mute from the journal, on this thread. Used
/// when the audio thread cannot do it.
fn unpark_here(journal: std::path::PathBuf) {
    system::com_init();
    let mut parker = Parker::new(journal);
    parker.tick(None);
}

/// cpal's streams and the COM objects stay on this thread.
fn audio_thread(
    rx: Receiver<Job>,
    controls: Controls,
    parked: Arc<Mutex<Parked>>,
    vmic_view: Arc<Mutex<VmicView>>,
    journal: std::path::PathBuf,
) {
    system::com_init();
    let mut parker = Parker::new(journal);
    let mut playing: Option<Playing> = None;
    let mut vmic = Vmic::default();
    let mut last_vmic = Instant::now();
    let mut last_park = Instant::now();
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Job::Vmic(want)) => {
                vmic.want = want;
                vmic.failed = None;
                vmic.tick(&controls, &vmic_view);
                last_vmic = Instant::now();
            }
            Ok(Job::Devices(reply)) => {
                let hide = hidden();
                let _ = reply.send(list_devices().map(|mut l| {
                    l.outputs
                        .retain(|o| !hide.iter().any(|h| !h.capture && h.name == *o));
                    l.inputs
                        .retain(|i| !hide.iter().any(|h| h.capture && h.name == *i));
                    l.default_output = l.default_output.filter(|d| l.outputs.contains(d));
                    l
                }));
            }
            Ok(Job::Apps(reply)) => {
                let sessions = system::sessions().unwrap_or_default();
                let _ = reply.send(park::running_apps(&sessions, &system::processes()));
            }
            Ok(Job::Sounding(reply)) => {
                let _ = reply.send(sounding());
            }
            Ok(Job::Start(spec, reply)) => {
                playing = None;
                last_park = Instant::now();
                let out = Router::start(&spec.output, &spec.routes, controls.clone())
                    .map(|router| playing = Some(Playing { router, spec }));
                // Right away either way: a new output may be the old parking output, still muted.
                park_apps(&mut parker, playing.as_mut(), &parked);
                let _ = reply.send(out);
            }
            Ok(Job::Plan(spec)) => {
                if let Some(p) = playing.as_mut() {
                    p.spec = spec;
                    park_apps(&mut parker, Some(p), &parked);
                }
            }
            Ok(Job::Stop(reply)) => {
                playing = None;
                park_apps(&mut parker, None, &parked);
                last_park = Instant::now();
                if let Some(r) = reply {
                    let _ = r.send(());
                }
            }
            Ok(Job::Shutdown(reply)) => {
                playing = None;
                vmic = Vmic::default();
                park_apps(&mut parker, None, &parked);
                let _ = reply.send(());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // By the clock, so a steady stream of requests never holds off parking.
        if last_park.elapsed() >= TICK {
            if playing.is_some() || parker.dirty() {
                park_apps(&mut parker, playing.as_mut(), &parked);
            }
            last_park = Instant::now();
        }
        if last_vmic.elapsed() >= TICK {
            vmic.tick(&controls, &vmic_view);
            last_vmic = Instant::now();
        }
    }
}

impl Mixer {
    /// Starts the audio thread, and the routes too if they were playing when FoxDen closed.
    pub fn start(shared: Shared) -> Mixer {
        let config = lock(&shared).config.audio.clone().tidy();
        let controls: Controls = Arc::new(Mutex::new(config.state()));
        let parked: Arc<Mutex<Parked>> = Arc::default();
        let vmic: Arc<Mutex<VmicView>> = Arc::default();
        let (tx, rx) = channel();
        let _ = tx.send(Job::Vmic(config.vmic()));
        let (c, p, v) = (controls.clone(), parked.clone(), vmic.clone());
        let journal = crate::config::dir().join(JOURNAL);
        thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || audio_thread(rx, c, p, v, journal))
            .expect("audio mixer thread");
        let resume = config.running;
        let mixer = Mixer {
            inner: Arc::new(Mutex::new(Inner {
                config,
                running: false,
                error: None,
                devices: None,
                apps: Vec::new(),
                follow: None,
                playing_on: None,
                tried: None,
            })),
            controls,
            parked,
            vmic,
            jobs: Arc::new(Mutex::new(tx)),
            shared,
        };
        if resume {
            let m = mixer.clone();
            thread::spawn(move || m.resume());
        }
        let m = mixer.clone();
        thread::spawn(move || {
            loop {
                thread::sleep(AUTO_EVERY);
                m.auto_assign();
            }
        });
        mixer
    }

    /// Audio devices can still be appearing just after sign-in, so a failed start is retried a few times.
    fn resume(&self) {
        for attempt in 1..=LAUNCH_TRIES {
            match self.play() {
                Ok(()) => return crate::log::info("audio", "mixer resumed"),
                Err(e) if attempt == LAUNCH_TRIES => {
                    crate::log::warn("audio", format!("mixer did not resume: {e}"))
                }
                Err(_) => thread::sleep(LAUNCH_WAIT),
            }
        }
    }

    /// Stops playing and puts parked apps and the parking output back. Called on exit.
    pub fn shutdown(&self) {
        let (tx, rx) = sync_channel(1);
        if !self.send(Job::Shutdown(tx)) || rx.recv_timeout(Duration::from_secs(5)).is_err() {
            crate::log::warn(
                "audio",
                "audio thread did not stop; putting apps back from here",
            );
            unpark_here(crate::config::dir().join(JOURNAL));
        }
    }

    fn guard(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, job: Job) -> bool {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(job)
            .is_ok()
    }

    fn ask<T>(&self, job: impl FnOnce(SyncSender<T>) -> Job) -> Result<T, String> {
        let (tx, rx) = sync_channel(1);
        if !self.send(job(tx)) {
            return Err("the audio thread has stopped".into());
        }
        rx.recv()
            .map_err(|_| "the audio thread has stopped".to_string())
    }

    fn devices(&self, rescan: bool) -> Result<(), String> {
        let apps = self.ask(Job::Apps)?;
        self.guard().apps = apps;
        if !rescan && self.guard().devices.is_some() {
            return Ok(());
        }
        let list = self.ask(Job::Devices)??;
        let mut inner = self.guard();
        if inner.config.output.is_empty() {
            inner.config.output = list.default_output.clone().unwrap_or_default();
        }
        inner.devices = Some(list);
        Ok(())
    }

    /// Starts or restarts the routes, keeping the error for the panel.
    fn play(&self) -> Result<(), String> {
        let spec = self.guard().spec();
        let output = spec.as_ref().ok().map(|s| s.output.clone());
        let out = match spec {
            Ok(spec) => self.ask(|tx| Job::Start(spec, tx)).and_then(|r| r),
            Err(e) => {
                self.send(Job::Stop(None));
                Err(e)
            }
        };
        let mut inner = self.guard();
        inner.playing_on = output.filter(|_| out.is_ok());
        inner.running = out.is_ok();
        inner.error = out.as_ref().err().cloned();
        out
    }

    fn stop(&self) {
        let (tx, rx) = sync_channel(1);
        if !self.send(Job::Stop(Some(tx))) || rx.recv_timeout(STOP_WAIT).is_err() {
            crate::log::warn(
                "audio",
                "audio thread did not answer; putting apps back from here",
            );
            unpark_here(crate::config::dir().join(JOURNAL));
        }
        let mut inner = self.guard();
        inner.running = false;
        inner.error = None;
    }

    fn controls(&self) -> MutexGuard<'_, MixState> {
        self.controls.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hands the chat mix to a headset's dial, as game and chat in 0..=100, or back to the slider.
    pub fn set_dial(&self, dial: Option<(u8, u8)>) {
        self.controls().dial = dial.map(|(g, c)| (f32::from(g) / 100.0, f32::from(c) / 100.0));
    }

    fn apply(&self, change: Change) -> Result<(), String> {
        let mut replan = false;
        let vmic_changed = matches!(change, Change::Route { .. } | Change::VirtualMic { .. });
        let restart = {
            let mut inner = self.guard();
            let running = inner.running;
            let cfg = &mut inner.config;
            match change {
                Change::Output { name } => {
                    if cfg
                        .channels
                        .iter()
                        .any(|c| c.on && c.loopback && c.source == name)
                    {
                        return Err(
                            "That output is a source; it would play back into itself".into()
                        );
                    }
                    cfg.output = name;
                    running
                }
                Change::Park { name } => {
                    cfg.park = name;
                    replan = running;
                    false
                }
                Change::Route {
                    channel,
                    source,
                    loopback,
                    on,
                } => {
                    if on && loopback && source == cfg.output {
                        return Err(format!(
                            "{} would play back into itself; pick another source or output",
                            channel.label()
                        ));
                    }
                    let ch = cfg.channel(channel);
                    let only_on = ch.source == source && ch.loopback == loopback;
                    (ch.source, ch.loopback, ch.on) = (source, loopback, on);
                    // Switching a channel with only apps on or off needs no new streams.
                    replan = running && only_on && ch.source.is_empty();
                    running && !replan
                }
                Change::Apps { channel, apps } => {
                    if channel.is_capture() {
                        return Err("The mic takes an input, not apps".into());
                    }
                    let taken: Vec<String> = apps.iter().map(|a| park::lower(a)).collect();
                    let before: Vec<String> = cfg
                        .channel(channel)
                        .apps
                        .iter()
                        .map(|a| park::lower(a))
                        .collect();
                    for c in cfg.channels.iter_mut().filter(|c| c.channel != channel) {
                        c.apps.retain(|a| !taken.contains(&park::lower(a)));
                    }
                    // A hand-placed app is no longer automatic; a removed one is not placed again.
                    let moved_here: Vec<&String> =
                        taken.iter().filter(|a| !before.contains(a)).collect();
                    cfg.auto.retain(|a| !moved_here.contains(&a));
                    cfg.no_auto.retain(|a| !moved_here.contains(&a));
                    cfg.no_auto
                        .extend(before.iter().filter(|a| !taken.contains(a)).cloned());
                    let ch = cfg.channel(channel);
                    if !apps.is_empty() && ch.apps.is_empty() && ch.source.is_empty() {
                        ch.on = true;
                    }
                    ch.apps = apps;
                    *cfg = std::mem::take(cfg).tidy();
                    replan = running;
                    false
                }
                Change::Volume { channel, volume } => {
                    let volume = finite(volume, 1.0).clamp(0.0, 1.0);
                    cfg.channel(channel).volume = volume;
                    self.controls().set_volume(channel, volume);
                    false
                }
                Change::Mute { channel, muted } => {
                    cfg.channel(channel).muted = muted;
                    self.controls().set_muted(channel, muted);
                    false
                }
                Change::Master { volume } => {
                    cfg.master = finite(volume, 1.0).clamp(0.0, 1.0);
                    self.controls().master = cfg.master;
                    false
                }
                Change::MasterMute { muted } => {
                    cfg.master_muted = muted;
                    self.controls().master_muted = muted;
                    false
                }
                Change::ChatMix { balance } => {
                    cfg.chat_mix = finite(balance, 0.0).clamp(-1.0, 1.0);
                    self.controls().chat_mix = cfg.chat_mix;
                    false
                }
                Change::Eq {
                    channel,
                    enabled,
                    mut bands,
                } => {
                    bands.truncate(dsp::MAX_BANDS);
                    let ch = cfg.channel(channel);
                    (ch.eq_on, ch.eq) = (enabled, bands.clone());
                    self.controls().set_eq(channel, enabled, bands);
                    false
                }
                Change::VirtualMic { on } => {
                    cfg.virtual_mic = on;
                    false
                }
                Change::Gate { on, threshold_db } => {
                    cfg.gate_on = on;
                    cfg.gate_db = finite(threshold_db, GATE_DB).clamp(-90.0, 0.0);
                    self.controls().gate = cfg.gate();
                    false
                }
                Change::AutoAssign { on } => {
                    cfg.auto_assign = on;
                    false
                }
                Change::AllowAuto { exe } => {
                    let key = park::lower(&exe);
                    cfg.no_auto.retain(|a| *a != key);
                    false
                }
                Change::ApplyProfile { channel, profile } => {
                    let p = profile.clean();
                    if p.kind != kind_of(channel) {
                        return Err(match p.kind {
                            Kind::Mic => "That is a mic profile; apply it to the mic".into(),
                            Kind::Output => {
                                "That profile is for an output channel, not the mic".into()
                            }
                        });
                    }
                    let ch = cfg.channel(channel);
                    if let Some(eq) = p.eq {
                        (ch.eq_on, ch.eq) = (eq.on, eq.bands.clone());
                        self.controls().set_eq(channel, eq.on, eq.bands);
                    }
                    if let Some(v) = p.volume {
                        ch.volume = v;
                        self.controls().set_volume(channel, v);
                    }
                    if let Some(g) = p.gate {
                        (cfg.gate_on, cfg.gate_db) = (g.on, g.db);
                        self.controls().gate = cfg.gate();
                    }
                    false
                }
                Change::SaveProfile { channel, name } => {
                    let (gate_on, gate_db) = (cfg.gate_on, cfg.gate_db);
                    let ch = cfg.channel(channel).clone();
                    let kind = kind_of(channel);
                    let p = Profile {
                        name,
                        kind,
                        builtin: false,
                        eq: Some(profiles::Eq {
                            on: ch.eq_on,
                            bands: ch.eq,
                        }),
                        volume: Some(ch.volume),
                        gate: (kind == Kind::Mic).then_some(Gate {
                            on: gate_on,
                            db: gate_db,
                        }),
                    }
                    .clean();
                    store(&mut cfg.profiles, p)?;
                    false
                }
                Change::AddProfile { profile } => {
                    store(&mut cfg.profiles, profile.clean())?;
                    false
                }
                Change::RenameProfile { from, to } => {
                    let to = profiles::clean_name(&to);
                    if to.is_empty() {
                        return Err("Give the profile a name".into());
                    }
                    if to != from
                        && (cfg.profiles.iter().any(|p| p.name == to)
                            || profiles::builtins().iter().any(|b| b.name == to))
                    {
                        return Err(format!("There is already a profile called {to}"));
                    }
                    let p = cfg
                        .profiles
                        .iter_mut()
                        .find(|p| p.name == from)
                        .ok_or("That profile is gone")?;
                    p.name = to;
                    false
                }
                Change::DeleteProfile { name } => {
                    cfg.profiles.retain(|p| p.name != name);
                    false
                }
                Change::Run { on } => {
                    cfg.running = on;
                    if !on {
                        drop(inner);
                        self.stop();
                        self.save();
                        return Ok(());
                    }
                    true
                }
            }
        };
        self.save();
        if vmic_changed {
            let want = self.guard().config.vmic();
            self.send(Job::Vmic(want));
        }
        if replan {
            match self.guard().spec() {
                Ok(spec) if !spec.routes.is_empty() || !spec.apps.is_empty() => {
                    self.send(Job::Plan(spec));
                }
                _ => {
                    // Nothing left to play.
                    self.stop();
                    self.guard().config.running = false;
                    self.save();
                }
            }
        }
        if restart {
            let out = self.play();
            let mut inner = self.guard();
            if inner.config.running != out.is_ok() {
                inner.config.running = out.is_ok();
                drop(inner);
                self.save();
            }
            return out;
        }
        Ok(())
    }

    /// Plays on the priority's pick, or on the configured output with `None`. A failed start is
    /// tried again on a later call while the mixer is meant to run.
    pub fn follow(&self, output: Option<String>) {
        let restart = {
            let mut inner = self.guard();
            inner.follow = output;
            let want = inner.config.running;
            let moved = inner.running && inner.playing_on.as_deref() != Some(inner.output());
            let retry = !inner.running && inner.tried.is_none_or(|t| t.elapsed() >= FOLLOW_RETRY);
            if want && (moved || retry && inner.follow.is_some()) {
                inner.tried = Some(Instant::now());
                Some(inner.output().to_string())
            } else {
                None
            }
        };
        let Some(name) = restart else { return };
        match self.play() {
            Ok(()) => crate::log::info("audio", format!("mixer output: {name}")),
            Err(e) => crate::log::warn("audio", format!("mixer did not move to {name}: {e}")),
        }
    }

    /// Outputs the priority must not pick: the chosen parking output and recorded outputs.
    pub fn avoid(&self) -> Vec<String> {
        let cfg = &self.guard().config;
        cfg.channels
            .iter()
            .filter(|c| c.on && c.loopback && !c.source.is_empty())
            .map(|c| c.source.clone())
            .chain((!cfg.park.is_empty()).then(|| cfg.park.clone()))
            .collect()
    }

    /// Sends apps that just started playing to their preset channel, while the mixer runs.
    fn auto_assign(&self) {
        {
            let inner = self.guard();
            if !inner.running || !inner.config.auto_assign {
                return;
            }
        }
        let Ok(now) = self.ask(Job::Sounding) else {
            return;
        };
        let replan = {
            let mut inner = self.guard();
            let cfg = &mut inner.config;
            let placed = cfg.apps_anywhere();
            let mut added = false;
            for (exe, path) in now {
                let key = park::lower(&exe);
                if placed.contains(&key) || cfg.no_auto.contains(&key) {
                    continue;
                }
                let Some(channel) = foxden_mixer::presets::preset(&exe, &path) else {
                    continue;
                };
                crate::log::info(
                    "audio",
                    format!("{exe} auto-assigned to {}", channel.label()),
                );
                let ch = cfg.channel(channel);
                ch.apps.push(exe);
                ch.on = true;
                cfg.auto.push(key);
                added = true;
            }
            added
        };
        if replan {
            self.save();
            if let Ok(spec) = self.guard().spec() {
                self.send(Job::Plan(spec));
            }
        }
    }

    pub fn priority(&self) -> crate::priority::PriorityConfig {
        self.guard().config.priority.clone()
    }

    pub fn set_priority(&self, p: crate::priority::PriorityConfig) {
        self.guard().config.priority = p;
        self.save();
    }

    /// Hands the settings to the config saver.
    fn save(&self) {
        let config = self.guard().config.clone();
        let mut state = lock(&self.shared);
        state.config.audio = config;
        state.changed = Some(Instant::now());
    }

    fn view(&self) -> View {
        let inner = self.guard();
        let parked = self
            .parked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let devices = inner.devices.clone().unwrap_or_default();
        View {
            curves: inner.config.channels.iter().map(curve).collect(),
            vmic: self.vmic.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            config: inner.config.clone(),
            profiles: profiles::builtins()
                .into_iter()
                .chain(inner.config.profiles.iter().cloned())
                .collect(),
            playing_on: inner.playing_on.clone().filter(|_| inner.running),
            following: inner.follow.is_some(),
            running: inner.running,
            error: inner.error.clone(),
            outputs: devices.outputs,
            inputs: devices.inputs,
            apps: inner.apps.clone(),
            app_states: if inner.running {
                parked.states
            } else {
                BTreeMap::new()
            },
            parked_on: parked.sink.filter(|_| inner.running),
            park_error: parked.error.filter(|_| inner.running),
            filters: FilterType::ALL
                .map(|f| Filter {
                    id: f,
                    label: f.label(),
                    gain: f.uses_gain(),
                })
                .into(),
        }
    }
}

/// The EQ's response in dB, from 20 Hz to 20 kHz on a log scale.
fn curve(c: &ChannelConfig) -> Vec<f32> {
    (0..CURVE_POINTS)
        .map(|i| {
            let f = dsp::MIN_FREQ * 1000f32.powf(i as f32 / (CURVE_POINTS - 1) as f32);
            if c.eq_on {
                dsp::response_db(&c.eq, 48_000.0, f)
            } else {
                0.0
            }
        })
        .collect()
}

#[derive(Serialize)]
pub struct Filter {
    id: FilterType,
    label: &'static str,
    gain: bool,
}

#[derive(Serialize)]
pub struct View {
    config: AudioConfig,
    /// The output it plays on, which the priority may have picked.
    playing_on: Option<String>,
    following: bool,
    profiles: Vec<Profile>,
    running: bool,
    error: Option<String>,
    outputs: Vec<String>,
    inputs: Vec<String>,
    /// Running apps with sound, to pick from.
    apps: Vec<RunningApp>,
    /// Each assigned app's state while playing, by lowercase executable name.
    app_states: BTreeMap<String, AppState>,
    parked_on: Option<String>,
    park_error: Option<String>,
    filters: Vec<Filter>,
    curves: Vec<Vec<f32>>,
    vmic: VmicView,
}

/// Lists the audio devices the first time, or again with `rescan`, and the apps with sound.
#[tauri::command(async)]
pub fn get_mixer(mixer: State<'_, Mixer>, rescan: bool) -> Result<View, String> {
    if let Err(e) = mixer.devices(rescan) {
        mixer.guard().error = Some(e);
    }
    Ok(mixer.view())
}

#[tauri::command(async)]
pub fn mixer_set(mixer: State<'_, Mixer>, change: Change) -> Result<View, String> {
    mixer.apply(change)?;
    Ok(mixer.view())
}

/// Uses a VB-CABLE the user installed from VB-Audio: names it through the service, then turns
/// the virtual mic on. FoxDen does not ship VB-Audio's files.
#[tauri::command(async)]
pub fn virtual_mic_install(mixer: State<'_, Mixer>) -> Result<View, String> {
    foxden_vcable::com_init();
    if !foxden_vcable::driver_present() {
        return Err("VB-CABLE is not installed yet. Install it from VB-Audio's page, then press Check again.".into());
    }
    let text = vmic_action("rename")?;
    crate::log::info("audio", format!("virtual mic: {text}"));
    mixer.apply(Change::VirtualMic { on: true })?;
    Ok(mixer.view())
}

/// Turns the virtual mic off and removes VB-CABLE if FoxDen installed it.
#[tauri::command(async)]
pub fn virtual_mic_remove(mixer: State<'_, Mixer>) -> Result<View, String> {
    mixer.apply(Change::VirtualMic { on: false })?;
    let text = vmic_action("remove")?;
    crate::log::info("audio", format!("virtual mic: {text}"));
    Ok(mixer.view())
}

fn vmic_action(action: &str) -> Result<String, String> {
    let client = foxden_client::Client::new(foxden_client::PIPE, Duration::from_secs(240));
    match client.virtual_mic(action) {
        Ok(text) => return Ok(text),
        Err(e) if e.starts_with("virtual mic:") => return Err(e),
        Err(_) => {}
    }
    let exe = crate::modules::install_dir()?.join("foxden-service.exe");
    match crate::modules::run_elevated(&exe, &format!("vcable {action}"))? {
        0 => Ok(format!("{action} done")),
        code => Err(format!("VB-CABLE {action} failed (exit code {code})")),
    }
}

/// Peak level of each channel since the last call, in [`Channel::ALL`] order.
#[tauri::command]
pub fn get_mixer_levels(mixer: State<'_, Mixer>) -> [f32; 5] {
    let mut c = mixer.controls();
    let peaks = Channel::ALL.map(|ch| c.peak(ch));
    c.peaks = [0.0; 5];
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routed(output: &str, source: &str, loopback: bool) -> AudioConfig {
        let mut c = AudioConfig {
            output: output.into(),
            ..AudioConfig::default()
        };
        let ch = c.channel(Channel::Game);
        (ch.source, ch.loopback, ch.on) = (source.into(), loopback, true);
        c
    }

    #[test]
    fn an_output_never_plays_into_itself() {
        assert!(routed("Speakers", "Speakers", true).spec().is_err());
        assert_eq!(
            routed("Speakers", "Speakers", false)
                .spec()
                .unwrap()
                .routes
                .len(),
            1
        );
        assert_eq!(
            routed("Speakers", "Game cable", true)
                .spec()
                .unwrap()
                .routes[0]
                .source,
            "Game cable"
        );
        assert!(AudioConfig::default().spec().is_err());
    }

    #[test]
    fn apps_alone_are_enough_to_play_and_go_by_lowercase_name() {
        let mut c = routed("Speakers", "", true);
        c.channel(Channel::Game).apps = vec!["Game.EXE".into()];
        c.channel(Channel::ChatRender).apps = vec!["chat.exe".into()];
        let spec = c.spec().unwrap();
        assert!(spec.routes.is_empty());
        assert_eq!(
            spec.apps.into_iter().collect::<Vec<_>>(),
            [("game.exe".to_string(), Channel::Game)]
        );
    }

    #[test]
    fn tidy_keeps_one_entry_per_channel_in_range() {
        let c = AudioConfig {
            channels: vec![
                ChannelConfig {
                    channel: Channel::Aux,
                    volume: 3.0,
                    eq: Vec::new(),
                    apps: vec!["a.exe".into(), "A.exe".into()],
                    ..ChannelConfig::default()
                },
                ChannelConfig {
                    channel: Channel::ChatCapture,
                    apps: vec!["b.exe".into()],
                    ..ChannelConfig::default()
                },
            ],
            chat_mix: f32::NAN,
            ..AudioConfig::default()
        }
        .tidy();
        assert_eq!(
            c.channels.iter().map(|x| x.channel).collect::<Vec<_>>(),
            Channel::ALL
        );
        assert_eq!(c.channels[3].volume, 1.0);
        assert_eq!(c.channels[3].eq.len(), dsp::MAX_BANDS);
        assert_eq!(c.channels[3].apps, ["a.exe"]);
        assert!(c.channels[4].apps.is_empty());
        assert_eq!(c.chat_mix, 0.0);
    }

    #[test]
    fn changes_read_from_the_ui() {
        let c: Change = serde_json::from_str(
            r#"{"kind":"eq","channel":"media","enabled":true,"bands":[{"enabled":true,"filter":"LowShelving","frequency":80,"gain_db":3,"q":0.7}]}"#,
        )
        .unwrap();
        assert!(
            matches!(c, Change::Eq { channel: Channel::Media, enabled: true, ref bands } if bands[0].filter == FilterType::LowShelving)
        );
        let c: Change = serde_json::from_str(
            r#"{"kind":"apps","channel":"chatRender","apps":["Discord.exe"]}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            Change::Apps {
                channel: Channel::ChatRender,
                apps: vec!["Discord.exe".into()]
            }
        );
    }
}
