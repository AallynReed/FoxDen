//! The mixer: channels fed by devices and by single apps, with volume, chat mix and EQ.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use foxden_mixer::Channel;
use foxden_mixer::dsp::{self, Band, FilterType};
use foxden_mixer::park::{self, AppState, Parker, Plan, RunningApp};
use foxden_mixer::router::{Controls, DeviceList, DeviceRoute, MixState, Router, list_devices};
use foxden_mixer::system;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::{Shared, lock};

const CURVE_POINTS: usize = 96;
const LAUNCH_TRIES: u32 = 6;
const LAUNCH_WAIT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_secs(1);
const JOURNAL: &str = "audio-parked.json";

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
        self
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
        s
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
}

/// What the audio thread last saw of the apps.
#[derive(Clone, Debug, Default)]
struct Parked {
    states: BTreeMap<String, AppState>,
    sink: Option<String>,
    error: Option<String>,
}

enum Job {
    Devices(SyncSender<Result<DeviceList, String>>),
    Apps(SyncSender<Vec<RunningApp>>),
    Start(Spec, SyncSender<Result<(), String>>),
    /// New app assignments or parking output while playing.
    Plan(Spec),
    Stop,
    Shutdown(SyncSender<()>),
}

struct Inner {
    config: AudioConfig,
    running: bool,
    error: Option<String>,
    devices: Option<DeviceList>,
    apps: Vec<RunningApp>,
}

#[derive(Clone)]
pub struct Mixer {
    inner: Arc<Mutex<Inner>>,
    controls: Controls,
    parked: Arc<Mutex<Parked>>,
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
            let endpoints = system::endpoints().unwrap_or_default();
            let recorded: Vec<&str> = p
                .spec
                .routes
                .iter()
                .filter(|r| r.source_is_output)
                .map(|r| r.source.as_str())
                .collect();
            match park::choose_sink(&endpoints, &p.spec.park, &p.spec.output, &recorded) {
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

/// cpal's streams and the COM objects stay on this thread.
fn audio_thread(
    rx: Receiver<Job>,
    controls: Controls,
    parked: Arc<Mutex<Parked>>,
    journal: std::path::PathBuf,
) {
    system::com_init();
    let mut parker = Parker::new(journal);
    let mut playing: Option<Playing> = None;
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Job::Devices(reply)) => {
                let _ = reply.send(list_devices());
            }
            Ok(Job::Apps(reply)) => {
                let sessions = system::sessions().unwrap_or_default();
                let _ = reply.send(park::running_apps(&sessions, &system::processes()));
            }
            Ok(Job::Start(spec, reply)) => {
                playing = None;
                let out = Router::start(&spec.output, &spec.routes, controls.clone())
                    .map(|router| playing = Some(Playing { router, spec }));
                if out.is_ok() {
                    park_apps(&mut parker, playing.as_mut(), &parked);
                }
                let _ = reply.send(out);
            }
            Ok(Job::Plan(spec)) => {
                if let Some(p) = playing.as_mut() {
                    p.spec = spec;
                    park_apps(&mut parker, Some(p), &parked);
                }
            }
            Ok(Job::Stop) => {
                playing = None;
                park_apps(&mut parker, None, &parked);
            }
            Ok(Job::Shutdown(reply)) => {
                playing = None;
                park_apps(&mut parker, None, &parked);
                let _ = reply.send(());
            }
            Err(RecvTimeoutError::Timeout) => {
                if playing.is_some() || parker.dirty() {
                    park_apps(&mut parker, playing.as_mut(), &parked);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

impl Mixer {
    /// Starts the audio thread, and the routes too if they were playing when FoxDen closed.
    pub fn start(shared: Shared) -> Mixer {
        let config = lock(&shared).config.audio.clone().tidy();
        let controls: Controls = Arc::new(Mutex::new(config.state()));
        let parked: Arc<Mutex<Parked>> = Arc::default();
        let (tx, rx) = channel();
        let (c, p) = (controls.clone(), parked.clone());
        let journal = crate::config::dir().join(JOURNAL);
        thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || audio_thread(rx, c, p, journal))
            .expect("audio mixer thread");
        let resume = config.running;
        let mixer = Mixer {
            inner: Arc::new(Mutex::new(Inner {
                config,
                running: false,
                error: None,
                devices: None,
                apps: Vec::new(),
            })),
            controls,
            parked,
            jobs: Arc::new(Mutex::new(tx)),
            shared,
        };
        if resume {
            let m = mixer.clone();
            thread::spawn(move || m.resume());
        }
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
        if self.send(Job::Shutdown(tx)) {
            let _ = rx.recv_timeout(Duration::from_secs(5));
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
        let spec = self.guard().config.spec();
        let out = match spec {
            Ok(spec) => self.ask(|tx| Job::Start(spec, tx)).and_then(|r| r),
            Err(e) => {
                self.send(Job::Stop);
                Err(e)
            }
        };
        let mut inner = self.guard();
        inner.running = out.is_ok();
        inner.error = out.as_ref().err().cloned();
        out
    }

    fn stop(&self) {
        self.send(Job::Stop);
        let mut inner = self.guard();
        inner.running = false;
        inner.error = None;
    }

    fn controls(&self) -> MutexGuard<'_, MixState> {
        self.controls.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn apply(&self, change: Change) -> Result<(), String> {
        let mut replan = false;
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
                    for c in cfg.channels.iter_mut().filter(|c| c.channel != channel) {
                        c.apps.retain(|a| !taken.contains(&park::lower(a)));
                    }
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
        if replan {
            match self.guard().config.spec() {
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
            config: inner.config.clone(),
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
