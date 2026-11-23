//! Poly / Plantronics headsets behind a BT600 adapter, over its vendor collection.

use std::time::Duration;

use foxden_battery::hid::Dev;

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x047F;
const BT600: u16 = 0x02EE;

pub fn model(pid: u16) -> Option<Model> {
    (pid == BT600).then_some(Model {
        vid: VID,
        pid,
        name: "Voyager 8200 UC",
        family: Family::Poly,
        sidetone: true,
        sidetone_max: 0,
        mic: false,
        anc: false,
        wireless: true,
    })
}

const REPORT: u8 = 0x07;
const TO_ADAPTER: u8 = 0x00;
const TO_HEADSET: u8 = 0x20;
const FROM_HEADSET: u8 = 0x02;
const HELLO: u8 = 0x01;
const GET: u8 = 0x02;
const GET_REPLY: u8 = 0x03;
const SET: u8 = 0x05;
const HELLO_ACK: u8 = 0x08;
const CHANGED: u8 = 0x0A;

const HELLO_ITEM: (u8, u8) = (0x01, 0x02);
const BATTERY: (u8, u8) = (0x0A, 0x1A);
const ONLINE_LED: (u8, u8) = (0x04, 0x09);
const SIDETONE: (u8, u8) = (0x04, 0x10);
const TONES: (u8, u8) = (0x04, 0x11);
const NOISE_LIMIT: (u8, u8) = (0x0F, 0x0E);

pub fn controls() -> Vec<(Control, Shape)> {
    vec![
        (
            Control::Sidetone,
            Shape::Choice {
                options: vec![(0, "Low"), (1, "Medium"), (2, "High")],
            },
        ),
        (Control::VolumeLimiter, Shape::Switch),
        (Control::VoicePrompts, Shape::Switch),
        (Control::BusyLight, Shape::Switch),
    ]
}

fn item(c: Control) -> Option<(u8, u8)> {
    Some(match c {
        Control::Sidetone => SIDETONE,
        Control::VolumeLimiter => NOISE_LIMIT,
        Control::VoicePrompts => TONES,
        Control::BusyLight => ONLINE_LED,
        _ => return None,
    })
}

fn packet(op: u8, (group, item): (u8, u8), addr: u8, value: Option<u8>) -> Vec<u8> {
    let mut p = vec![
        REPORT,
        0x01,
        0x01,
        0x10,
        if op == GET { 0x06 } else { 0x07 },
        addr,
        0x00,
        0x00,
        op,
        group,
        item,
    ];
    p.extend(value);
    p
}

fn answers(r: &[u8], op: u8, (group, item): (u8, u8), addr: u8) -> bool {
    r.len() > 10
        && r[..4] == [REPORT, 0x01, 0x01, 0x10]
        && r[5] == addr
        && r[8] == op
        && r[9] == group
        && r[10] == item
}

/// Battery value `[level][level count][charging]`.
fn battery(r: &[u8]) -> Option<(u8, bool)> {
    let (level, count, charging) = (*r.get(11)? as u32, *r.get(12)? as u32, *r.get(13)?);
    (count >= 2 && level < count).then(|| ((level * 100 / (count - 1)) as u8, charging != 0))
}

fn setting(c: Control, r: &[u8]) -> Option<(Control, i32)> {
    let max = if c == Control::Sidetone { 2 } else { 1 };
    r.get(11).filter(|v| **v <= max).map(|v| (c, *v as i32))
}

const WAIT: Duration = Duration::from_millis(1000);

fn exchange(dev: &Dev, req: &[u8], op: u8, what: (u8, u8), from: u8) -> Option<Vec<u8>> {
    if !dev.write(req) {
        return None;
    }
    dev.read_until(WAIT, |r| answers(r, op, what, from).then(|| r.to_vec()))
}

/// Opens the adapter's and then the headset's session. `None` when the adapter is silent,
/// `Some(false)` when the headset is off or out of range.
fn session(dev: &Dev) -> Option<bool> {
    dev.flush();
    dev.write(&[0x06, 0x01]);
    exchange(
        dev,
        &packet(HELLO, HELLO_ITEM, TO_ADAPTER, None),
        HELLO_ACK,
        HELLO_ITEM,
        TO_ADAPTER,
    )?;
    let req = packet(HELLO, HELLO_ITEM, TO_HEADSET, None);
    Some(exchange(dev, &req, HELLO_ACK, HELLO_ITEM, FROM_HEADSET).is_some())
}

fn get(dev: &Dev, what: (u8, u8)) -> Option<Vec<u8>> {
    exchange(
        dev,
        &packet(GET, what, TO_HEADSET, None),
        GET_REPLY,
        what,
        FROM_HEADSET,
    )
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let Some(on) = session(&dev) else {
        return Some(Reading::default());
    };
    let mut status = Status {
        connected: on,
        ..Status::default()
    };
    let mut values = Vec::new();
    if on {
        if let Some((pct, charging)) = get(&dev, BATTERY).as_deref().and_then(battery) {
            status.battery = Some(pct.min(100));
            status.charging = charging;
        }
        if settings {
            for (c, _) in controls() {
                // A headset that ignores one GET ignores the rest; each costs the full wait.
                let Some(r) = item(c).and_then(|w| get(&dev, w)) else {
                    break;
                };
                values.extend(setting(c, &r));
            }
        }
    }
    Some(Reading {
        status: Some(status),
        values,
    })
}

/// The SET frame for `c` at `v`.
fn set_frame(c: Control, v: i32) -> Result<Vec<u8>, String> {
    let what = item(c).ok_or_else(|| format!("this headset has no {} setting", c.id()))?;
    if !controls().iter().any(|(k, s)| *k == c && s.accepts(v)) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    Ok(packet(SET, what, TO_HEADSET, Some(v as u8)))
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    let req = set_frame(c, v)?;
    let dev = Dev::open(&f.coll).ok_or("the headset's adapter could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    match session(&dev) {
        None => return Err("the headset's adapter did not answer".into()),
        Some(false) => return Err("the headset is switched off".into()),
        Some(true) => {}
    }
    let what = (req[9], req[10]);
    exchange(&dev, &req, CHANGED, what, FROM_HEADSET)
        .map(|_| ())
        .ok_or_else(|| "the headset did not take the setting".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        assert_eq!(
            packet(HELLO, HELLO_ITEM, TO_HEADSET, None),
            vec![0x07, 1, 1, 0x10, 0x07, 0x20, 0, 0, 0x01, 0x01, 0x02]
        );
        assert_eq!(
            packet(GET, SIDETONE, TO_HEADSET, None),
            vec![0x07, 1, 1, 0x10, 0x06, 0x20, 0, 0, 0x02, 0x04, 0x10]
        );
        assert_eq!(
            set_frame(Control::Sidetone, 2).unwrap(),
            vec![0x07, 1, 1, 0x10, 0x07, 0x20, 0, 0, 0x05, 0x04, 0x10, 0x02]
        );
        assert_eq!(
            set_frame(Control::BusyLight, 1).unwrap(),
            vec![0x07, 1, 1, 0x10, 0x07, 0x20, 0, 0, 0x05, 0x04, 0x09, 0x01]
        );
        assert_eq!(
            set_frame(Control::VoicePrompts, 0).unwrap(),
            vec![0x07, 1, 1, 0x10, 0x07, 0x20, 0, 0, 0x05, 0x04, 0x11, 0x00]
        );
        assert_eq!(
            set_frame(Control::VolumeLimiter, 1).unwrap(),
            vec![0x07, 1, 1, 0x10, 0x07, 0x20, 0, 0, 0x05, 0x0F, 0x0E, 0x01]
        );
        assert!(set_frame(Control::Sidetone, 3).is_err());
        assert!(set_frame(Control::VolumeLimiter, 2).is_err());
        assert!(set_frame(Control::AutoOff, 10).is_err());
        assert!(model(0x02F7).is_none());
    }

    #[test]
    fn replies() {
        let ack = [0x07, 1, 1, 0x10, 0x07, 0x02, 0, 0, 0x0A, 0x04, 0x10, 0x01];
        assert!(answers(&ack, CHANGED, SIDETONE, FROM_HEADSET));
        assert!(!answers(&ack, CHANGED, BATTERY, FROM_HEADSET));
        let event = [
            0x07, 1, 1, 0x10, 0x0A, 0x00, 0, 0, 0x0A, 0x0E, 0x1E, 0x07, 0x02, 0x0C,
        ];
        assert!(!answers(&event, CHANGED, (0x0E, 0x1E), FROM_HEADSET));
        let fragment = [
            0x07, 1, 0x07, 0x11, 0x9C, 0x02, 0, 0, 0x09, 0x00, 0x36, 0x00,
        ];
        assert!(!answers(&fragment, 0x09, (0x00, 0x36), FROM_HEADSET));
        let b = [
            0x07, 1, 1, 0x10, 0x0C, 0x02, 0, 0, 0x03, 0x0A, 0x1A, 0x07, 0x0B, 0x00, 0x02, 0xEE,
            0x01,
        ];
        assert!(answers(&b, GET_REPLY, BATTERY, FROM_HEADSET));
        assert_eq!(battery(&b), Some((70, false)));
        let mut r = vec![0x07, 1, 1, 0x10, 0x07, 0x02, 0, 0, 0x03, 0x04, 0x10, 0x01];
        assert_eq!(setting(Control::Sidetone, &r), Some((Control::Sidetone, 1)));
        r[11] = 3;
        assert_eq!(setting(Control::Sidetone, &r), None);
        r[11] = 2;
        assert_eq!(setting(Control::BusyLight, &r), None);
        assert_eq!(setting(Control::BusyLight, &r[..11]), None);
    }
}
