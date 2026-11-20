//! The standalone mixer: Sonar's volume, chat mix and EQ rules without SteelSeries software.

use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use steelseries_gg::Channel;
use steelseries_gg::dsp::{self, Band, FilterType};
use steelseries_gg::router::{Controls, DeviceList, MixState, RouteSpec, Router, list_devices};
use tauri::State;

use crate::state::{Shared, lock};

const CURVE_POINTS: usize = 96;
const LAUNCH_TRIES: u32 = 6;
const LAUNCH_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    pub channel: Channel,
    pub source: String,
    /// The source is an output: what plays on it is recorded.
    pub loopback: bool,
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
            channels: Channel::ALL.map(ChannelConfig::new).to_vec(),
            master: 1.0,
            master_muted: false,
            chat_mix: 0.0,
            running: false,
        }
    }
}

impl AudioConfig {
    /// One entry per channel in Sonar's order, values in range.
    fn tidy(mut self) -> Self {
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

    /// The routes to play, refusing any that would feed an output back into itself.
    fn specs(&self) -> Result<Vec<RouteSpec>, String> {
        if self.output.is_empty() {
            return Err("Pick an output to play on".into());
        }
        let specs: Vec<RouteSpec> = self
            .channels
            .iter()
            .filter(|c| c.on && !c.source.is_empty())
            .map(|c| RouteSpec {
                channel: c.channel,
                source: c.source.clone(),
                source_is_output: c.loopback,
                output: self.output.clone(),
            })
            .collect();
        if let Some(r) = specs
            .iter()
            .find(|r| r.source_is_output && r.source == r.output)
        {
            return Err(format!(
                "{} would play back into itself; pick another source or output",
                r.channel.label()
            ));
        }
        if specs.is_empty() {
            return Err("Turn on at least one channel with a source".into());
        }
        Ok(specs)
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
    Route {
        channel: Channel,
        source: String,
        loopback: bool,
        on: bool,
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

enum Job {
    Devices(SyncSender<Result<DeviceList, String>>),
    Start(Vec<RouteSpec>, SyncSender<Result<(), String>>),
    Stop,
}

struct Inner {
    config: AudioConfig,
    running: bool,
    error: Option<String>,
    devices: Option<DeviceList>,
}

#[derive(Clone)]
pub struct Mixer {
    inner: Arc<Mutex<Inner>>,
    controls: Controls,
    jobs: Arc<Mutex<Sender<Job>>>,
    shared: Shared,
}

/// cpal's streams are not `Send`, so the router lives and dies on this thread.
fn audio_thread(rx: Receiver<Job>, controls: Controls) {
    let mut router: Option<Router> = None;
    for job in rx {
        match job {
            Job::Devices(reply) => {
                let _ = reply.send(list_devices().map_err(|e| e.to_string()));
            }
            Job::Start(specs, reply) => {
                router = None;
                let out = Router::start(&specs, controls.clone()).map(|r| router = Some(r));
                let _ = reply.send(out.map_err(|e| e.to_string()));
            }
            Job::Stop => router = None,
        }
    }
    drop(router);
}

impl Mixer {
    /// Starts the audio thread, and the routes too if they were playing when FoxDen closed.
    pub fn start(shared: Shared) -> Mixer {
        let config = lock(&shared).config.audio.clone().tidy();
        let controls: Controls = Arc::new(Mutex::new(config.state()));
        let (tx, rx) = channel();
        let c = controls.clone();
        thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || audio_thread(rx, c))
            .expect("audio mixer thread");
        let resume = config.running;
        let mixer = Mixer {
            inner: Arc::new(Mutex::new(Inner {
                config,
                running: false,
                error: None,
                devices: None,
            })),
            controls,
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
                Ok(()) => return crate::log::info("audio", "standalone mixer resumed"),
                Err(e) if attempt == LAUNCH_TRIES => {
                    crate::log::warn("audio", format!("standalone mixer did not resume: {e}"))
                }
                Err(_) => thread::sleep(LAUNCH_WAIT),
            }
        }
    }

    fn guard(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn ask<T>(&self, job: impl FnOnce(SyncSender<T>) -> Job) -> Result<T, String> {
        let (tx, rx) = sync_channel(1);
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(job(tx))
            .map_err(|_| "the audio thread has stopped".to_string())?;
        rx.recv()
            .map_err(|_| "the audio thread has stopped".to_string())
    }

    fn devices(&self, rescan: bool) -> Result<(), String> {
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
        let specs = self.guard().config.specs();
        let out = match specs {
            Ok(specs) => self.ask(|tx| Job::Start(specs, tx)).and_then(|r| r),
            Err(e) => {
                let _ = self
                    .jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Job::Stop);
                Err(e)
            }
        };
        let mut inner = self.guard();
        inner.running = out.is_ok();
        inner.error = out.as_ref().err().cloned();
        out
    }

    fn stop(&self) {
        let _ = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Job::Stop);
        let mut inner = self.guard();
        inner.running = false;
        inner.error = None;
    }

    fn controls(&self) -> MutexGuard<'_, MixState> {
        self.controls.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn apply(&self, change: Change) -> Result<(), String> {
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
                    (ch.source, ch.loopback, ch.on) = (source, loopback, on);
                    running
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
        let devices = inner.devices.clone().unwrap_or_default();
        View {
            curves: inner.config.channels.iter().map(curve).collect(),
            config: inner.config.clone(),
            running: inner.running,
            error: inner.error.clone(),
            outputs: devices.outputs,
            inputs: devices.inputs,
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
    filters: Vec<Filter>,
    curves: Vec<Vec<f32>>,
}

/// Lists the audio devices the first time, or again with `rescan`.
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

/// Peak level of each channel since the last call, in Sonar's order.
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
        assert!(routed("Speakers", "Speakers", true).specs().is_err());
        assert_eq!(
            routed("Speakers", "Speakers", false).specs().unwrap().len(),
            1
        );
        assert_eq!(
            routed("Speakers", "Game cable", true).specs().unwrap()[0].source,
            "Game cable"
        );
        assert!(AudioConfig::default().specs().is_err());
    }

    #[test]
    fn tidy_keeps_one_entry_per_channel_in_range() {
        let c = AudioConfig {
            channels: vec![ChannelConfig {
                channel: Channel::Aux,
                volume: 3.0,
                eq: Vec::new(),
                ..ChannelConfig::default()
            }],
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
    }
}
