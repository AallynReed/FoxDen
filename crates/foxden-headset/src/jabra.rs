//! Jabra headsets on their own cable or behind a Link dongle, over the vendor collection.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use foxden_battery::hid::Dev;

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x0B0E;
const DONGLE: u8 = 0x04;
const CABLE: u8 = 0x08;

/// Product id, name, whether a Link dongle, usage of the 0xFF00 collection.
const MODELS: &[(u16, &str, bool, u16)] = &[
    (0x24C7, "Link 380", true, 0x01),
    (0x24C8, "Link 380", true, 0x01),
    (0x24C9, "Link 380", true, 0x01),
    (0x24CA, "Link 380", true, 0x01),
    (0x2E50, "Link 390", true, 0x05),
    (0x2E51, "Link 390", true, 0x05),
    (0x2E56, "Link 390", true, 0x05),
    (0x2E57, "Link 390", true, 0x05),
    (0x2517, "Evolve2 65 Flex", false, 0x01),
    (0x2518, "Evolve2 65 Flex", false, 0x01),
    (0x2519, "Evolve2 65 Flex", false, 0x01),
    (0x2546, "Evolve3 75", false, 0x05),
    (0x2547, "Evolve3 75", false, 0x05),
    (0x2548, "Evolve3 75", false, 0x05),
];

pub fn model(pid: u16) -> Option<Model> {
    let &(pid, name, dongle, _) = MODELS.iter().find(|m| m.0 == pid)?;
    Some(Model {
        vid: VID,
        pid,
        name,
        family: Family::Jabra,
        sidetone: true,
        sidetone_max: 0,
        mic: false,
        anc: false,
        wireless: dongle,
    })
}

pub fn usage(pid: u16) -> u16 {
    MODELS.iter().find(|m| m.0 == pid).map_or(0x05, |m| m.3)
}

fn address(m: &Model) -> u8 {
    if m.wireless { DONGLE } else { CABLE }
}

const GET: u8 = 1;
const SET: u8 = 2;
const REPLY: u8 = 3;

const BATTERY: (u8, u8) = (0x12, 0x02);
const BUSY: (u8, u8) = (0x12, 0x08);
const INTELLITONE: (u8, u8) = (0x13, 0x01);
const PROMPTS: (u8, u8) = (0x13, 0x3A);
const SIDETONE: (u8, u8) = (0x13, 0x7C);
const INACTIVITY: (u8, u8) = (0x13, 0x90);

/// Sidetone gain steps in dB; control values 1..=6, 0 is muted.
const SIDETONE_DB: [i8; 6] = [-9, -6, -3, 0, 3, 6];
const INTELLITONE_ON: u8 = 0x03;

pub fn controls() -> Vec<(Control, Shape)> {
    vec![
        (Control::Sidetone, Shape::Steps { min: 0, max: 6 }),
        (
            Control::AutoOff,
            Shape::Minutes {
                max: 255 * 5,
                presets: vec![0, 5, 10, 15, 30, 60, 120],
            },
        ),
        (Control::VolumeLimiter, Shape::Switch),
        (Control::VoicePrompts, Shape::Switch),
        (Control::BusyLight, Shape::Switch),
    ]
}

fn frame(addr: u8, seq: u8, kind: u8, (cmd, sub): (u8, u8), payload: &[u8]) -> Vec<u8> {
    let mut f = vec![
        0x05,
        addr,
        0x00,
        seq,
        (kind << 6) | (6 + payload.len() as u8),
        cmd,
        sub,
    ];
    f.extend_from_slice(payload);
    f
}

#[derive(Debug, PartialEq)]
enum Answer {
    Data(Vec<u8>),
    Ack,
    Nak(u8),
}

/// The reply to request `seq` for `(cmd, sub)`; other reports are `None`.
fn answer(r: &[u8], seq: u8, (cmd, sub): (u8, u8)) -> Option<Answer> {
    if r.len() < 7 || r[0] != 0x05 || r[3] != seq || r[4] >> 6 != REPLY {
        return None;
    }
    match r[5] {
        0xFF => Some(Answer::Ack),
        0xFE => Some(Answer::Nak(r[6])),
        c if c == cmd && r[6] == sub => {
            let len = (r[4] & 0x3F) as usize;
            Some(Answer::Data(r.get(7..len + 1).unwrap_or_default().to_vec()))
        }
        _ => None,
    }
}

fn offline(nak: u8) -> bool {
    matches!(nak, 0xF3..=0xF5)
}

fn battery(p: &[u8]) -> Option<(Option<u8>, bool)> {
    (p.len() >= 2).then(|| ((p[1] <= 100).then_some(p[1]), p[0] & 0x01 != 0))
}

fn setting(c: Control, p: &[u8]) -> Option<(Control, i32)> {
    let v = match c {
        Control::Sidetone => {
            let db = *p.get(1)? as i8;
            let i = SIDETONE_DB.iter().position(|d| *d == db)?;
            if p[0] == 1 { 0 } else { i as i32 + 1 }
        }
        Control::AutoOff => *p.first()? as i32 * 5,
        Control::VolumeLimiter => match p.first()? {
            0 => 0,
            &INTELLITONE_ON => 1,
            _ => return None,
        },
        Control::VoicePrompts | Control::BusyLight => match p.first()? {
            v @ 0..=1 => *v as i32,
            _ => return None,
        },
        _ => return None,
    };
    Some((c, v))
}

fn item(c: Control) -> Option<(u8, u8)> {
    Some(match c {
        Control::Sidetone => SIDETONE,
        Control::AutoOff => INACTIVITY,
        Control::VolumeLimiter => INTELLITONE,
        Control::VoicePrompts => PROMPTS,
        Control::BusyLight => BUSY,
        _ => return None,
    })
}

/// What a SET of `c` to `v` carries. Muting keeps the gain, `db_now`, that was set.
fn payload(c: Control, v: i32, db_now: Option<i8>) -> Result<Vec<u8>, String> {
    if !controls().iter().any(|(k, s)| *k == c && s.accepts(v)) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    Ok(match c {
        Control::Sidetone if v == 0 => vec![0x01, db_now.unwrap_or(0) as u8],
        Control::Sidetone => vec![0x00, SIDETONE_DB[v as usize - 1] as u8],
        Control::AutoOff if v % 5 != 0 => return Err("auto-off goes in steps of 5 minutes".into()),
        Control::AutoOff => vec![(v / 5) as u8],
        Control::VolumeLimiter => vec![if v == 1 { INTELLITONE_ON } else { 0 }],
        Control::BusyLight => vec![v as u8, 0x00],
        _ => vec![v as u8],
    })
}

static SEQ: AtomicU8 = AtomicU8::new(0xA0);
const WAIT: Duration = Duration::from_millis(700);

fn ask(dev: &Dev, addr: u8, kind: u8, what: (u8, u8), payload: &[u8]) -> Option<Answer> {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    dev.flush();
    if !dev.write(&frame(addr, seq, kind, what, payload)) {
        return None;
    }
    dev.read_until(WAIT, |r| answer(r, seq, what))
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let addr = address(&f.model);
    let status = match ask(&dev, addr, GET, BATTERY, &[]) {
        Some(Answer::Data(p)) => {
            let (battery, charging) = battery(&p).unwrap_or_default();
            Status {
                connected: true,
                battery,
                charging,
                ..Status::default()
            }
        }
        Some(Answer::Nak(n)) if offline(n) => Status::default(),
        Some(_) => Status {
            connected: true,
            ..Status::default()
        },
        None => return Some(Reading::default()),
    };
    let mut values = Vec::new();
    if settings && status.connected {
        for (c, _) in controls() {
            let Some(what) = item(c) else { continue };
            if let Some(Answer::Data(p)) = ask(&dev, addr, GET, what, &[]) {
                values.extend(setting(c, &p));
            }
        }
    }
    Some(Reading {
        status: Some(status),
        values,
    })
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    let what = item(c).ok_or_else(|| format!("{} has no {} setting", f.model.name, c.id()))?;
    payload(c, v, None)?;
    let dev = Dev::open(&f.coll).ok_or("the headset could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    let addr = address(&f.model);
    let db_now = match (c, v) {
        (Control::Sidetone, 0) => match ask(&dev, addr, GET, SIDETONE, &[]) {
            Some(Answer::Data(p)) => p.get(1).map(|d| *d as i8),
            _ => None,
        },
        _ => None,
    };
    match ask(&dev, addr, SET, what, &payload(c, v, db_now)?) {
        Some(Answer::Ack) => Ok(()),
        Some(Answer::Nak(n)) if offline(n) => Err("the headset is switched off".into()),
        Some(Answer::Nak(0xF6 | 0xFF)) => Err(format!("this headset has no {} setting", c.id())),
        Some(_) => Err("the headset refused the setting".into()),
        None => Err("the headset did not answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_and_collections() {
        let link = model(0x2E51).unwrap();
        assert_eq!((link.name, address(&link)), ("Link 390", DONGLE));
        assert_eq!(usage(0x24C7), 0x01);
        assert_eq!(usage(0x2548), 0x05);
        assert_eq!(address(&model(0x2519).unwrap()), CABLE);
        assert!(model(0x1016).is_none());
    }

    #[test]
    fn frames() {
        assert_eq!(
            frame(DONGLE, 0x21, GET, BATTERY, &[]),
            vec![0x05, 0x04, 0x00, 0x21, 0x46, 0x12, 0x02]
        );
        assert_eq!(
            frame(
                CABLE,
                0x30,
                SET,
                SIDETONE,
                &payload(Control::Sidetone, 6, None).unwrap()
            ),
            vec![0x05, 0x08, 0x00, 0x30, 0x88, 0x13, 0x7C, 0x00, 0x06]
        );
        let p = |c, v| payload(c, v, None).unwrap();
        assert_eq!(p(Control::Sidetone, 1), vec![0x00, 0xF7]);
        assert_eq!(p(Control::Sidetone, 4), vec![0x00, 0x00]);
        assert_eq!(
            payload(Control::Sidetone, 0, Some(-3)).unwrap(),
            vec![0x01, 0xFD]
        );
        assert_eq!(p(Control::Sidetone, 0), vec![0x01, 0x00]);
        assert_eq!(p(Control::AutoOff, 15), vec![3]);
        assert_eq!(p(Control::AutoOff, 0), vec![0]);
        assert_eq!(p(Control::AutoOff, 1275), vec![255]);
        assert_eq!(p(Control::VolumeLimiter, 1), vec![0x03]);
        assert_eq!(p(Control::VolumeLimiter, 0), vec![0x00]);
        assert_eq!(p(Control::VoicePrompts, 1), vec![0x01]);
        assert_eq!(p(Control::BusyLight, 1), vec![0x01, 0x00]);
        assert!(payload(Control::AutoOff, 7, None).is_err());
        assert!(payload(Control::AutoOff, 1280, None).is_err());
        assert!(payload(Control::Sidetone, 7, None).is_err());
        assert!(payload(Control::VoicePrompts, 2, None).is_err());
        assert!(payload(Control::MicVolume, 1, None).is_err());
    }

    #[test]
    fn replies() {
        let r = [0x05, 0x00, 0x04, 0x21, 0xC8, 0x12, 0x02, 0x01, 0x4B, 0, 0];
        let Some(Answer::Data(p)) = answer(&r, 0x21, BATTERY) else {
            panic!()
        };
        assert_eq!(battery(&p), Some((Some(75), true)));
        assert_eq!(answer(&r, 0x22, BATTERY), None);
        assert_eq!(answer(&r, 0x21, SIDETONE), None);
        let nak = [0x05, 0x00, 0x04, 0x21, 0xC6, 0xFE, 0xF3, 0];
        assert_eq!(answer(&nak, 0x21, BATTERY), Some(Answer::Nak(0xF3)));
        assert!(offline(0xF3) && offline(0xF5) && !offline(0xFF));
        let ack = [0x05, 0x00, 0x08, 0x30, 0xC6, 0xFF, 0x7C, 0];
        assert_eq!(answer(&ack, 0x30, SIDETONE), Some(Answer::Ack));
        let event = [0x05, 0x00, 0x04, 0x21, 0x08, 0x12, 0x02, 0x01, 0x4B];
        assert_eq!(answer(&event, 0x21, BATTERY), None);
    }

    #[test]
    fn settings_read_back() {
        assert_eq!(
            setting(Control::Sidetone, &[0x00, 0x03]),
            Some((Control::Sidetone, 5))
        );
        assert_eq!(
            setting(Control::Sidetone, &[0x01, 0x03]),
            Some((Control::Sidetone, 0))
        );
        assert_eq!(
            setting(Control::Sidetone, &[0x00, 0xF7]),
            Some((Control::Sidetone, 1))
        );
        assert_eq!(setting(Control::Sidetone, &[0x00, 0x05]), None);
        assert_eq!(
            setting(Control::AutoOff, &[6]),
            Some((Control::AutoOff, 30))
        );
        assert_eq!(
            setting(Control::VolumeLimiter, &[3]),
            Some((Control::VolumeLimiter, 1))
        );
        assert_eq!(setting(Control::VolumeLimiter, &[5]), None);
        assert_eq!(
            setting(Control::VoicePrompts, &[0]),
            Some((Control::VoicePrompts, 0))
        );
        assert_eq!(setting(Control::VoicePrompts, &[2]), None);
        assert_eq!(
            setting(Control::BusyLight, &[1]),
            Some((Control::BusyLight, 1))
        );
        assert_eq!(setting(Control::BusyLight, &[]), None);
    }
}
