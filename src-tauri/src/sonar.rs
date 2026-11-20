//! SteelSeries Sonar's mixer, driven from a worker thread: its calls block for seconds when GG is down.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use steelseries_gg::sonar::{ChatMix, ClassicRedirection, DataFlow, VolumeSettings};
use steelseries_gg::{Channel, Endpoints, Error, Mode, SonarClient, StreamMix};
use tauri::{AppHandle, Emitter, State};

use crate::state::{Shared, lock};

const POLL: Duration = Duration::from_secs(2);
const DEVICE_POLLS: u32 = 5;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Mode {
        mode: Mode,
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
    StreamVolume {
        mix: StreamMix,
        channel: Channel,
        volume: f32,
    },
    StreamMute {
        mix: StreamMix,
        channel: Channel,
        muted: bool,
    },
    StreamMaster {
        mix: StreamMix,
        volume: f32,
    },
    StreamMasterMute {
        mix: StreamMix,
        muted: bool,
    },
    ChatMix {
        balance: f32,
    },
    Device {
        channel: Channel,
        id: String,
    },
}

impl Change {
    /// Queued changes with the same key replace each other, so a slider drag sends its last value.
    fn key(&self) -> String {
        match self {
            Change::Mode { .. } => "mode".into(),
            Change::Volume { channel, .. } => format!("v{channel:?}"),
            Change::Mute { channel, .. } => format!("m{channel:?}"),
            Change::Master { .. } => "v".into(),
            Change::MasterMute { .. } => "m".into(),
            Change::StreamVolume { mix, channel, .. } => format!("sv{mix:?}{channel:?}"),
            Change::StreamMute { mix, channel, .. } => format!("sm{mix:?}{channel:?}"),
            Change::StreamMaster { mix, .. } => format!("sv{mix:?}"),
            Change::StreamMasterMute { mix, .. } => format!("sm{mix:?}"),
            Change::ChatMix { .. } => "cm".into(),
            Change::Device { channel, .. } => format!("d{channel:?}"),
        }
    }

    fn apply(&self, s: &SonarClient) -> Result<(), Error> {
        match self {
            Change::Mode { mode } => s.set_mode(*mode),
            Change::Volume { channel, volume } => s.set_volume(*channel, volume.clamp(0.0, 1.0)),
            Change::Mute { channel, muted } => s.set_mute(*channel, *muted),
            Change::Master { volume } => s.set_master_volume(volume.clamp(0.0, 1.0)),
            Change::MasterMute { muted } => s.set_master_mute(*muted),
            Change::StreamVolume {
                mix,
                channel,
                volume,
            } => s.set_stream_volume(*mix, *channel, volume.clamp(0.0, 1.0)),
            Change::StreamMute {
                mix,
                channel,
                muted,
            } => s.set_stream_mute(*mix, *channel, *muted),
            Change::StreamMaster { mix, volume } => {
                s.set_stream_master_volume(*mix, volume.clamp(0.0, 1.0))
            }
            Change::StreamMasterMute { mix, muted } => s.set_stream_master_mute(*mix, *muted),
            Change::ChatMix { balance } => s.set_chat_mix(balance.clamp(-1.0, 1.0)).map(|_| ()),
            Change::Device { channel, id } => s.set_classic_redirection(*channel, id),
        }
    }
}

/// Keeps the last of each key, in the order they last arrived.
fn coalesce(changes: Vec<Change>) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    for c in changes {
        let key = c.key();
        out.retain(|o| o.key() != key);
        out.push(c);
    }
    out
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Device {
    id: String,
    name: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct View {
    /// `off`, `searching`, `offline` or `online`.
    state: &'static str,
    problem: Option<String>,
    mode: Option<Mode>,
    classic: Option<VolumeSettings>,
    streamer: Option<VolumeSettings>,
    chat_mix: Option<ChatMix>,
    redirections: Vec<ClassicRedirection>,
    outputs: Vec<Device>,
    inputs: Vec<Device>,
    /// The last change Sonar refused.
    error: Option<String>,
}

enum Msg {
    Watch(bool),
    Set(Change),
}

pub struct Sonar {
    tx: Mutex<Sender<Msg>>,
    view: Arc<Mutex<View>>,
}

pub fn start(app: AppHandle, shared: Shared) -> Sonar {
    let (tx, rx) = channel();
    let view = Arc::new(Mutex::new(View {
        state: "off",
        ..View::default()
    }));
    let v = view.clone();
    thread::Builder::new()
        .name("sonar".into())
        .spawn(move || run(rx, app, shared, v))
        .expect("sonar thread");
    Sonar {
        tx: Mutex::new(tx),
        view,
    }
}

impl Sonar {
    fn send(&self, msg: Msg) -> Result<(), String> {
        self.tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(msg)
            .map_err(|_| "the Sonar thread has stopped".to_string())
    }
}

fn run(rx: Receiver<Msg>, app: AppHandle, shared: Shared, view: Arc<Mutex<View>>) {
    let mut ep: Option<Endpoints> = None;
    let mut watching = false;
    let mut last: Option<Instant> = None;
    let mut polls = 0u32;
    loop {
        let first = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut changes = Vec::new();
        for m in first.into_iter().chain(rx.try_iter()) {
            match m {
                Msg::Watch(on) => {
                    if on && !watching {
                        last = None;
                    }
                    watching = on;
                }
                Msg::Set(c) => changes.push(c),
            }
        }
        if !lock(&shared).config.settings.steelseries_gg {
            ep = None;
            polls = 0;
            if put(
                &view,
                View {
                    state: "off",
                    ..View::default()
                },
            ) {
                emit(&app, &view);
            }
            continue;
        }
        let mut wrote = false;
        for c in coalesce(changes) {
            let result = sonar(&mut ep).and_then(|s| c.apply(&s).map_err(|e| failed(&mut ep, e)));
            if let Ok(mut v) = view.lock() {
                v.error = result.err();
            }
            wrote = true;
        }
        let due = last.is_none_or(|t| t.elapsed() >= POLL);
        if (watching && due) || wrote {
            if view.lock().is_ok_and(|v| v.state == "off") {
                put(
                    &view,
                    View {
                        state: "searching",
                        ..View::default()
                    },
                );
                emit(&app, &view);
            }
            refresh(&mut ep, &view, polls.is_multiple_of(DEVICE_POLLS));
            polls += 1;
            last = Some(Instant::now());
            emit(&app, &view);
        }
    }
}

/// Replaces the view, saying whether that changed it.
fn put(view: &Mutex<View>, next: View) -> bool {
    let mut v = view.lock().unwrap_or_else(|e| e.into_inner());
    let changed = v.state != next.state;
    *v = next;
    changed
}

fn emit(app: &AppHandle, view: &Mutex<View>) {
    let v = view.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let _ = app.emit("sonar", v);
}

fn sonar(ep: &mut Option<Endpoints>) -> Result<SonarClient, String> {
    if ep.is_none() {
        *ep = Some(Endpoints::discover().map_err(|e| offline(&e))?);
    }
    ep.as_ref()
        .and_then(Endpoints::sonar)
        .ok_or_else(|| "Sonar is off in SteelSeries GG".to_string())
}

/// A call that could not reach Sonar drops the endpoints, which change every time GG starts.
fn failed(ep: &mut Option<Endpoints>, e: Error) -> String {
    match e {
        Error::Offline(_) => {
            *ep = None;
            offline(&e)
        }
        Error::Status { code: 403, .. } => {
            "Sonar refused the change; it is read-only right now".into()
        }
        e => e.to_string(),
    }
}

fn offline(e: &Error) -> String {
    match e {
        Error::Offline(_) | Error::Device(_) => "SteelSeries GG is not running".into(),
        e => format!("SteelSeries GG did not answer: {e}"),
    }
}

fn refresh(ep: &mut Option<Endpoints>, view: &Mutex<View>, devices: bool) {
    let mut next = view.lock().map(|v| v.clone()).unwrap_or_default();
    let found = sonar(ep).and_then(|s| {
        s.classic_volumes()
            .map(|v| (s, v))
            .map_err(|e| failed(ep, e))
    });
    let (s, classic) = match found {
        Ok(x) => x,
        Err(problem) => {
            put(
                view,
                View {
                    state: "offline",
                    problem: Some(problem),
                    error: next.error,
                    ..View::default()
                },
            );
            return;
        }
    };
    next.state = "online";
    next.problem = None;
    next.classic = Some(classic);
    next.mode = s.mode().ok();
    next.streamer = s.streamer_volumes().ok();
    next.chat_mix = s.chat_mix().ok();
    next.redirections = s.classic_redirections().unwrap_or_default();
    if devices || next.outputs.is_empty() {
        let list = |flow| {
            s.audio_devices(flow)
                .unwrap_or_default()
                .into_iter()
                .filter(|d| !d.is_vad)
                .map(|d| Device {
                    id: d.id,
                    name: d.friendly_name,
                })
                .collect::<Vec<_>>()
        };
        next.outputs = list(DataFlow::Render);
        next.inputs = list(DataFlow::Capture);
    }
    put(view, next);
}

#[tauri::command]
pub fn get_sonar(sonar: State<'_, Sonar>) -> View {
    sonar.view.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Polls Sonar every couple of seconds while the Audio tab is shown.
#[tauri::command]
pub fn sonar_watch(sonar: State<'_, Sonar>, on: bool) -> Result<(), String> {
    sonar.send(Msg::Watch(on))
}

#[tauri::command]
pub fn sonar_set(sonar: State<'_, Sonar>, change: Change) -> Result<(), String> {
    sonar.send(Msg::Set(change))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_sends_its_last_value() {
        let v = |volume| Change::Volume {
            channel: Channel::Game,
            volume,
        };
        let mute = Change::Mute {
            channel: Channel::Game,
            muted: true,
        };
        let chat = Change::Volume {
            channel: Channel::ChatRender,
            volume: 0.2,
        };
        let out = coalesce(vec![v(0.1), mute.clone(), v(0.2), chat.clone(), v(0.3)]);
        assert_eq!(out, [mute, chat, v(0.3)]);
    }

    #[test]
    fn changes_read_from_the_ui() {
        let c: Change = serde_json::from_str(
            r#"{"kind":"stream_volume","mix":"monitoring","channel":"chatRender","volume":0.5}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            Change::StreamVolume {
                mix: StreamMix::Monitoring,
                channel: Channel::ChatRender,
                volume: 0.5
            }
        );
        let c: Change = serde_json::from_str(r#"{"kind":"mode","mode":"stream"}"#).unwrap();
        assert_eq!(c, Change::Mode { mode: Mode::Stream });
    }
}
