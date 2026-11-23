//! Settings of SteelSeries wireless headsets over their own HID protocol, of Corsair headsets
//! as Bragi properties, and of other brands' headsets through their own modules.

mod logitech;
mod arctis;
mod epos;
mod jabra;
mod poly;
pub mod hyperx;
pub mod proto;
pub mod turtlebeach;

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use foxden_battery::Bragi;
use foxden_battery::hid::{Coll, Dev, enumerate};
pub use proto::{Control, EQ_BANDS, EQ_CUSTOM, EQ_MAX, EQ_MIN, Family, Model, Save, Shape, Status};
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
        if c.out_len == 0 {
            continue;
        }
        let Some(model) = proto::any_model(c.vid, c.pid) else {
            continue;
        };
        let fits = if model.family == Family::Bragi {
            Bragi::fits(&c)
        } else if model.family == Family::Logitech {
            logitech::fits(&c, &model)
        } else if model.family == Family::HyperX {
            hyperx::fits(&model, &c)
        } else if model.family == Family::TurtleBeach {
            turtlebeach::fits(&c)
        } else {
            match model.collection() {
                (_, Some((page, usage))) => c.page == page && c.usage == usage,
                (iface, None) => c.iface == iface,
            }
        };
        let key = format!("{:04X}:{:04X}", c.vid, c.pid);
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
    match f.model.family {
        Family::Bragi => return bragi::read(f, true),
        Family::Epos => return epos::read(f, true),
        Family::Jabra => return jabra::read(f, true),
        Family::Poly => return poly::read(f, true),
        _ => {}
    }
    if f.model.family == Family::Logitech {
        return logitech::read(f, true);
    }
    if f.model.family == Family::HyperX {
        return hyperx::read(f, true);
    }
    if f.model.family == Family::TurtleBeach {
        return turtlebeach::read(f);
    }
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
    match f.model.family {
        Family::Bragi => return bragi::read(f, false)?.status,
        Family::Epos => return slow_status(f, || epos::read(f, false)?.status),
        Family::Jabra => return slow_status(f, || jabra::read(f, false)?.status),
        Family::Poly => return slow_status(f, || poly::read(f, false)?.status),
        _ => {}
    }
    if f.model.family == Family::Logitech {
        return logitech::read(f, false)?.status;
    }
    if f.model.family == Family::HyperX {
        return hyperx::read(f, false)?.status;
    }
    if f.model.family == Family::TurtleBeach {
        return turtlebeach::status(f);
    }
    wait_ready(&f.key);
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    exchange(&dev, &f.model, &f.model.status_request()).and_then(|r| proto::status(&f.model, &r))
}

/// Last status of headsets that take a round trip or more per look, and when it was read.
static SLOW: Mutex<Vec<(String, Instant, Option<Status>)>> = Mutex::new(Vec::new());
const SLOW_EVERY: Duration = Duration::from_secs(10);

fn slow_status(f: &Found, ask: impl FnOnce() -> Option<Status>) -> Option<Status> {
    let mut slow = SLOW.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, at, s)) = slow.iter().find(|(k, _, _)| *k == f.key)
        && at.elapsed() < SLOW_EVERY
    {
        return s.clone();
    }
    let s = ask();
    slow.retain(|(k, _, _)| *k != f.key);
    slow.push((f.key.clone(), Instant::now(), s.clone()));
    s
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
    match f.model.family {
        Family::Bragi => return bragi::set(f, c, v),
        Family::Epos => return epos::set(f, c, v),
        Family::Jabra => return jabra::set(f, c, v),
        Family::Poly => return poly::set(f, c, v),
        _ => {}
    }
    if f.model.family == Family::Logitech {
        return logitech::set(f, c, v);
    }
    if f.model.family == Family::HyperX {
        return hyperx::set(f, c, v);
    }
    if f.model.family == Family::TurtleBeach {
        return turtlebeach::send(f, f.model.set(c, v)?);
    }
    send(f, f.model.set(c, v)?, f.model.save(Some(c)))
}

/// Custom EQ gains in half-dB steps.
pub fn set_eq(f: &Found, bands: &[i32]) -> Result<(), String> {
    if f.model.family == Family::TurtleBeach {
        return turtlebeach::send(f, f.model.eq(bands)?);
    }
    send(f, f.model.eq(bands)?, f.model.save(None))
}

/// Base stations with ChatMix reports switched on, and whether FoxDen is quitting.
static CHATMIX_ON: Mutex<(Vec<(String, Coll)>, bool)> = Mutex::new((Vec::new(), false));
const CHATMIX_READ: Duration = Duration::from_millis(10);

fn chatmix_write(dev: &Dev, m: &Model, on: bool) -> bool {
    let Some(frame) = m.chatmix_switch(on) else {
        return false;
    };
    let _turn = foxden_core::hid_turn::take();
    dev.write(&frame)
}

/// Keeps a base station's ChatMix reports on while it lives, and the dial's last position.
pub struct ChatMix {
    pub key: String,
    model: Model,
    dev: Dev,
    /// The interface's other collections: the dial may report on any of them.
    others: Vec<Dev>,
    connected: bool,
    last: Option<(u8, u8)>,
}

impl ChatMix {
    /// `None` for models without ChatMix reports, or when the base did not take the switch.
    pub fn open(f: &Found) -> Option<Self> {
        f.model.chatmix_switch(true)?;
        let c = Self {
            key: f.key.clone(),
            model: f.model,
            dev: Dev::open(&f.coll)?,
            others: enumerate()
                .iter()
                .filter(|c| {
                    (c.vid, c.pid, c.iface) == (f.coll.vid, f.coll.pid, f.coll.iface)
                        && c.path != f.coll.path
                })
                .filter_map(Dev::open_read_only)
                .collect(),
            connected: true,
            last: None,
        };
        c.enable().then_some(c)
    }

    /// Switches the reports on again, as after the base or the PC slept.
    pub fn enable(&self) -> bool {
        let mut on = CHATMIX_ON.lock().unwrap_or_else(|e| e.into_inner());
        if on.1 || !chatmix_write(&self.dev, &self.model, true) {
            return false;
        }
        if !on.0.iter().any(|(k, _)| *k == self.key) {
            on.0.push((self.key.clone(), self.dev.coll.clone()));
        }
        true
    }

    /// Reads what the base pushed since the last call. A headset coming back on gets the reports
    /// switched on again.
    pub fn poll(&mut self, connected: bool) -> Option<(u8, u8)> {
        if connected && !self.connected {
            self.enable();
        }
        self.connected = connected;
        for dev in std::iter::once(&self.dev).chain(&self.others) {
            for _ in 0..64 {
                let Some(r) = dev.read(CHATMIX_READ) else {
                    break;
                };
                if let Some(v) = proto::chatmix_report(&self.model, &r) {
                    self.last = Some(v);
                }
            }
        }
        self.last
    }
}

impl Drop for ChatMix {
    fn drop(&mut self) {
        let mut on = CHATMIX_ON.lock().unwrap_or_else(|e| e.into_inner());
        on.0.retain(|(k, _)| *k != self.key);
        chatmix_write(&self.dev, &self.model, false);
    }
}

/// Switches ChatMix reports off on every base that has them on, and keeps them off. Called on exit.
pub fn chatmix_off() {
    let mut on = CHATMIX_ON.lock().unwrap_or_else(|e| e.into_inner());
    on.1 = true;
    for (_, coll) in on.0.drain(..) {
        if let (Some(m), Some(dev)) = (proto::any_model(coll.vid, coll.pid), Dev::open(&coll)) {
            chatmix_write(&dev, &m, false);
        }
    }
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

mod bragi {
    use super::*;

    const BATTERY: u8 = 0x0F;
    const CHARGE: u8 = 0x10;
    const SIDETONE_MUTED: u8 = 0x46;
    const SIDETONE: u8 = 0x47;
    const MIC: u8 = 0x8F;
    const ANC: u8 = 0xD1;

    /// The headset behind a receiver answers as 0x09; a cabled one, or the headset itself, as 0x08.
    /// The receiver also answers as 0x08, so a wireless model counts there only with a battery.
    fn target(ch: &mut Bragi, m: &Model) -> Option<(u8, Option<u32>)> {
        if m.wireless {
            for t in [0x09, 0x08] {
                if let Some(level) = ch.get(t, BATTERY).filter(|l| *l <= 1000) {
                    return Some((t, Some(level)));
                }
            }
            None
        } else {
            ch.get(0x08, 0x13)
                .map(|_| (0x08, ch.get(0x08, BATTERY).filter(|l| *l <= 1000)))
        }
    }

    pub fn read(f: &Found, settings: bool) -> Option<Reading> {
        let mut ch = Bragi::open(&f.coll)?;
        let Some((t, level)) = target(&mut ch, &f.model) else {
            return Some(Reading {
                status: Some(Status::default()),
                values: Vec::new(),
            });
        };
        let charge = level.and(ch.get(t, CHARGE));
        let mut values = Vec::new();
        if settings {
            let m = &f.model;
            if m.sidetone {
                let muted = ch.get(t, SIDETONE_MUTED) == Some(1);
                let level = ch.get(t, SIDETONE).filter(|v| *v <= m.sidetone_max as u32);
                if muted {
                    values.push((Control::Sidetone, 0));
                } else if let Some(v) = level {
                    values.push((Control::Sidetone, v as i32));
                }
            }
            if m.mic
                && let Some(v) = ch.get(t, MIC).filter(|v| *v <= 100)
            {
                values.push((Control::MicVolume, v as i32));
            }
            if m.anc
                && let Some(v) = ch.get(t, ANC).filter(|v| *v <= 1)
            {
                values.push((Control::Anc, v as i32));
            }
        }
        Some(Reading {
            status: Some(Status {
                connected: true,
                battery: level.map(|l| ((l + 5) / 10).min(100) as u8),
                charging: matches!(charge, Some(1 | 3)),
                chatmix: None,
                values: Vec::new(),
            }),
            values,
        })
    }

    pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
        let shape = f
            .model
            .controls()
            .into_iter()
            .find(|(k, _)| *k == c)
            .map(|(_, s)| s);
        if !shape.is_some_and(|s| s.accepts(v)) {
            return Err(format!("{v} is out of range for {}", c.id()));
        }
        let mut ch = Bragi::open(&f.coll).ok_or("the headset could not be opened")?;
        let (t, _) = target(&mut ch, &f.model).ok_or("the headset is switched off")?;
        let v = v as u32;
        let took = match c {
            Control::Sidetone if v == 0 => ch.set(t, SIDETONE_MUTED, 1),
            Control::Sidetone => ch.set(t, SIDETONE_MUTED, 0) && ch.set(t, SIDETONE, v),
            Control::MicVolume => ch.set(t, MIC, v),
            Control::Anc => ch.set(t, ANC, v),
            _ => return Err(format!("{} has no {} setting", f.model.name, c.id())),
        };
        if took {
            Ok(())
        } else {
            Err("the headset did not take the setting".into())
        }
    }
}
