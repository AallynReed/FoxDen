//! Headset settings for SteelSeries headsets, through FoxDen's own HID code.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use foxden_headset::{Control, EQ_MAX, EQ_MIN, Found, Shape};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{Shared, lock};

/// What FoxDen last set on a headset, for values it cannot read back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub values: BTreeMap<Control, i32>,
    pub eq: Option<Vec<i32>>,
}

#[derive(Serialize)]
pub struct ControlView {
    id: Control,
    shape: Shape,
    value: Option<i32>,
    /// Read from the headset, not remembered.
    read: bool,
}

#[derive(Serialize)]
pub struct EqView {
    bands: Option<Vec<i32>>,
    min: i32,
    max: i32,
}

#[derive(Serialize)]
pub struct View {
    key: String,
    name: String,
    /// The receiver answered.
    answered: bool,
    connected: bool,
    battery: Option<u8>,
    charging: bool,
    chatmix: Option<(u8, u8)>,
    controls: Vec<ControlView>,
    eq: Option<EqView>,
}

/// Wait after the last change before the headset is told to store its settings.
const SETTLE: Duration = Duration::from_millis(1500);

static PENDING: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn view(f: &Found, saved: &Saved) -> View {
    let reading = foxden_headset::read(f).unwrap_or_default();
    let status = reading.status.clone().unwrap_or_default();
    let controls = f
        .model
        .controls()
        .into_iter()
        .map(|(id, shape)| {
            let got = reading.values.iter().find(|(c, _)| *c == id).map(|v| v.1);
            ControlView {
                id,
                value: got.or_else(|| saved.values.get(&id).copied()),
                read: got.is_some(),
                shape,
            }
        })
        .collect();
    View {
        key: f.key.clone(),
        name: format!("SteelSeries {}", f.model.name),
        answered: reading.status.is_some(),
        connected: status.connected,
        battery: status.battery,
        charging: status.charging,
        chatmix: status.chatmix,
        controls,
        eq: f.model.has_eq().then(|| EqView {
            bands: saved.eq.clone(),
            min: EQ_MIN,
            max: EQ_MAX,
        }),
    }
}

#[tauri::command]
pub async fn headset_list(shared: State<'_, Shared>) -> Result<Vec<View>, String> {
    let saved = lock(&shared).config.headsets.clone();
    tauri::async_runtime::spawn_blocking(move || {
        foxden_headset::find()
            .iter()
            .map(|f| view(f, saved.get(&f.key).unwrap_or(&Saved::default())))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

fn found(key: &str) -> Result<Found, String> {
    foxden_headset::find()
        .into_iter()
        .find(|f| f.key == key)
        .ok_or_else(|| "the headset's receiver is not plugged in".into())
}

fn check_on(f: &Found) -> Result<(), String> {
    match foxden_headset::read(f).and_then(|r| r.status) {
        Some(s) if !s.connected => Err("the headset is switched off".into()),
        _ => Ok(()),
    }
}

/// Stores the settings once no change has come for [`SETTLE`].
fn save_later(f: Found) {
    if f.model.settled_save().is_none() {
        return;
    }
    let stamp = Instant::now();
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(f.key.clone(), stamp);
    thread::spawn(move || {
        thread::sleep(SETTLE);
        let last = PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|p| p.get(&f.key).copied());
        if last != Some(stamp) {
            return;
        }
        if let Err(e) = foxden_headset::save(&f) {
            crate::log::warn("headset", format!("{}: {e}", f.model.name));
        }
    });
}

fn remember(shared: &Shared, key: &str, change: impl FnOnce(&mut Saved)) {
    let mut state = lock(shared);
    change(state.config.headsets.entry(key.to_string()).or_default());
    state.changed = Some(Instant::now());
}

#[tauri::command]
pub async fn headset_set(
    shared: State<'_, Shared>,
    key: String,
    control: Control,
    value: i32,
) -> Result<(), String> {
    let f = tauri::async_runtime::spawn_blocking({
        let key = key.clone();
        move || -> Result<Found, String> {
            let f = found(&key)?;
            check_on(&f)?;
            foxden_headset::set(&f, control, value)?;
            Ok(f)
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::log::info(
        "headset",
        format!("{}: {} set to {value}", f.model.name, control.id()),
    );
    remember(&shared, &key, |s| {
        s.values.insert(control, value);
    });
    save_later(f);
    Ok(())
}

#[tauri::command]
pub async fn headset_eq(
    shared: State<'_, Shared>,
    key: String,
    bands: Vec<i32>,
) -> Result<(), String> {
    let sent = bands.clone();
    tauri::async_runtime::spawn_blocking({
        let key = key.clone();
        move || -> Result<(), String> {
            let f = found(&key)?;
            check_on(&f)?;
            foxden_headset::set_eq(&f, &sent)
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    remember(&shared, &key, |s| {
        s.eq = Some(bands);
        s.values
            .insert(Control::EqPreset, foxden_headset::EQ_CUSTOM);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_round_trips() {
        let mut s = Saved::default();
        s.values.insert(Control::Sidetone, 2);
        s.values.insert(Control::AutoOff, 30);
        s.eq = Some(vec![0, -4, 6, 0, 0, 0, 0, 0, 0, 20]);
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"sidetone\":2"));
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), s);
    }
}

/// A headset's ChatMix dial: game and chat, 0..=100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Dial {
    game: u8,
    chat: u8,
}

static DIAL: Mutex<Option<Dial>> = Mutex::new(None);

/// How often the dial is read, and how often headsets are looked for.
const DIAL_EVERY: Duration = Duration::from_millis(250);
const FIND_EVERY: Duration = Duration::from_secs(5);

/// Follows the ChatMix dial of a connected headset. While one is on, it drives the mixer's chat mix.
pub fn spawn_dial(app: AppHandle) {
    thread::spawn(move || {
        let mut found: Vec<Found> = Vec::new();
        let mut looked: Option<Instant> = None;
        loop {
            if looked.is_none_or(|t| t.elapsed() >= FIND_EVERY) {
                found = foxden_headset::find();
                looked = Some(Instant::now());
            }
            let statuses: Vec<_> = found.iter().map(foxden_headset::status).collect();
            crate::priority::set_headsets(
                found
                    .iter()
                    .zip(&statuses)
                    .map(|(f, s)| (f.key.clone(), s.as_ref().map(|s| s.connected))),
            );
            let now = statuses
                .iter()
                .flatten()
                .filter(|s| s.connected)
                .find_map(|s| s.chatmix)
                .map(|(game, chat)| Dial { game, chat });
            let changed = {
                let mut last = DIAL.lock().unwrap_or_else(|e| e.into_inner());
                std::mem::replace(&mut *last, now) != now
            };
            if changed {
                if let Some(mixer) = app.try_state::<crate::mixer::Mixer>() {
                    mixer.set_dial(now.map(|d| (d.game, d.chat)));
                }
                let _ = app.emit("chatmix", now);
            }
            thread::sleep(if found.is_empty() {
                FIND_EVERY
            } else {
                DIAL_EVERY
            });
        }
    });
}

#[tauri::command]
pub fn headset_dial() -> Option<Dial> {
    *DIAL.lock().unwrap_or_else(|e| e.into_inner())
}
