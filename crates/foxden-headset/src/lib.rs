//! Settings of SteelSeries wireless headsets, sent over their own HID protocol.

pub mod proto;

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use foxden_battery::hid::{Coll, Dev, enumerate};
pub use proto::{Control, EQ_BANDS, EQ_CUSTOM, EQ_MAX, EQ_MIN, Model, Save, Shape, Status};

const VID: u16 = 0x1038;
const WAIT: Duration = Duration::from_secs(1);

/// A supported headset's receiver or base station.
#[derive(Clone, Debug)]
pub struct Found {
    /// "1038:22A1".
    pub key: String,
    pub model: Model,
    coll: Coll,
}

pub fn find() -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for c in enumerate() {
        if c.vid != VID || c.out_len == 0 {
            continue;
        }
        let Some(model) = proto::model(c.pid) else {
            continue;
        };
        let (iface, usage) = model.collection();
        let fits = match usage {
            Some((page, usage)) => c.page == page && c.usage == usage,
            None => c.iface == iface,
        };
        let key = format!("{VID:04X}:{:04X}", c.pid);
        if fits && !out.iter().any(|f| f.key == key) {
            out.push(Found {
                key,
                model,
                coll: c,
            });
        }
    }
    out
}

/// Sends `req` and waits for the reply that starts with the same command.
fn exchange(dev: &Dev, m: &Model, req: &[u8]) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(req) {
        return None;
    }
    let head = m.reply_head(req);
    dev.read_until(WAIT, |r| r.starts_with(head).then(|| r.to_vec()))
}

/// What one look at a headset found.
#[derive(Clone, Debug, Default)]
pub struct Reading {
    /// `None` when the receiver did not answer.
    pub status: Option<Status>,
    /// Settings read back from the headset.
    pub values: Vec<(Control, i32)>,
}

pub fn read(f: &Found) -> Option<Reading> {
    wait_ready(&f.key);
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let status = exchange(&dev, &f.model, &f.model.status_request())
        .and_then(|r| proto::status(&f.model, &r));
    let mut values = Vec::new();
    if let Some(s) = status.as_ref().filter(|s| s.connected) {
        values.extend(s.values.iter().copied());
        for req in f.model.reads() {
            if let Some(r) = exchange(&dev, &f.model, &req) {
                values.extend(proto::settings(&f.model, &r));
            }
        }
    }
    Some(Reading { status, values })
}

/// Just the status: battery, power and the ChatMix dial. Cheap enough to ask often.
pub fn status(f: &Found) -> Option<Status> {
    wait_ready(&f.key);
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    exchange(&dev, &f.model, &f.model.status_request()).and_then(|r| proto::status(&f.model, &r))
}

fn send(f: &Found, frames: Vec<Vec<u8>>, save: Save) -> Result<(), String> {
    wait_ready(&f.key);
    let dev = Dev::open(&f.coll).ok_or("the headset's receiver could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    for frame in frames.iter().chain(match &save {
        Save::Now(frame) => Some(frame),
        _ => None,
    }) {
        if !dev.write(frame) {
            return Err("the headset's receiver did not take the setting".into());
        }
    }
    Ok(())
}

/// Sets one control. A settled save, if the model has one, is left to [`save`].
pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    send(f, f.model.set(c, v)?, f.model.save(Some(c)))
}

/// Custom EQ gains in half-dB steps.
pub fn set_eq(f: &Found, bands: &[i32]) -> Result<(), String> {
    send(f, f.model.eq(bands)?, f.model.save(None))
}

/// When each headset takes commands again after a save.
static BUSY: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

fn wait_ready(key: &str) {
    let until = BUSY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| k == key)
        .map(|b| b.1);
    if let Some(left) = until.and_then(|u| u.checked_duration_since(Instant::now())) {
        thread::sleep(left);
    }
}

/// Stores the settings in the headset, for models that save once changes settle.
pub fn save(f: &Found) -> Result<(), String> {
    let Some(frame) = f.model.settled_save() else {
        return Ok(());
    };
    wait_ready(&f.key);
    let dev = Dev::open(&f.coll).ok_or("the headset's receiver could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    if !dev.write(&frame) {
        return Err("the headset did not take the save".into());
    }
    let until = Instant::now() + Duration::from_millis(proto::SAVE_BUSY_MS);
    let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    busy.retain(|(k, _)| *k != f.key);
    busy.push((f.key.clone(), until));
    Ok(())
}
