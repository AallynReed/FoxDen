//! Audeze Maxwell and Maxwell 2 dongles: report 06 out, `05 type len body` messages in report 07.

use std::thread::sleep;
use std::time::{Duration, Instant};

use foxden_battery::hid::Dev;

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x3329;
const MAXWELL: [u16; 2] = [0x4B19, 0x4B18];
const MAXWELL_2: [u16; 2] = [0x4B29, 0x4B28];

pub fn model(pid: u16) -> Option<Model> {
    let name = if MAXWELL.contains(&pid) {
        "Maxwell"
    } else if MAXWELL_2.contains(&pid) {
        "Maxwell 2"
    } else {
        return None;
    };
    Some(Model {
        vid: VID,
        pid,
        name,
        family: Family::Audeze,
        sidetone: true,
        sidetone_max: SIDETONE_MAX,
        mic: false,
        anc: false,
        wireless: true,
    })
}

pub fn collection(m: &Model) -> (i32, Option<(u16, u16)>) {
    let iface = if MAXWELL.contains(&m.pid) { 0 } else { 5 };
    (iface, Some((0xFF13, 0x0001)))
}

const SIDETONE_MAX: u8 = 31;
const PRESETS: [(i32, &str); 10] = [
    (1, "Audeze"),
    (2, "Treble Boost"),
    (3, "Bass Boost"),
    (4, "Immersive"),
    (5, "Competition"),
    (6, "Footsteps"),
    (7, "Custom 1"),
    (8, "Custom 2"),
    (9, "Custom 3"),
    (10, "Custom 4"),
];
const OFF_MINUTES: [i32; 10] = [5, 10, 15, 30, 45, 60, 90, 120, 240, 360];
const OFF_NAMES: [&str; 10] = [
    "5 min", "10 min", "15 min", "30 min", "45 min", "60 min", "90 min", "120 min", "240 min",
    "360 min",
];

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let mut off = vec![(0, "Never")];
    off.extend(OFF_MINUTES.into_iter().zip(OFF_NAMES));
    let mut out = vec![
        (
            Control::Sidetone,
            Shape::Steps {
                min: 0,
                max: SIDETONE_MAX as i32,
            },
        ),
        (Control::AutoOff, Shape::Choice { options: off }),
        (
            Control::EqPreset,
            Shape::Choice {
                options: PRESETS.to_vec(),
            },
        ),
        (
            Control::NoiseGate,
            Shape::Choice {
                options: vec![(0, "Off"), (1, "Low"), (2, "High")],
            },
        ),
        (Control::VoicePrompts, Shape::Switch),
    ];
    if MAXWELL.contains(&m.pid) {
        out.push((Control::VolumeLimiter, Shape::Switch));
    }
    out
}

const REPORT_OUT: u8 = 0x06;
const REPORT_IN: u8 = 0x07;
const ROUTE: u8 = 0x80;
const FRAGMENT: usize = 59;
const REQUEST: u8 = 0x5A;
const RESPONSE: u8 = 0x5B;
const RESULT: u8 = 0x5D;

const KEY_PRESET: u8 = 0x00;
const KEY_PROMPTS: u8 = 0x20;
const KEY_GATE: u8 = 0x24;
const KEY_LIMITER: u8 = 0x28;
const KEY_SIDETONE: u8 = 0x2C;
const KEY_SIDETONE_MAX: u8 = 0x2D;
const PARAM_OFF: u8 = 0x01;
const PARAM_SIDETONE_ON: u8 = 0x07;
const PARAM_MIX: u8 = 0x0B;
const LIMITER_ON: u8 = 0x88;
const LIMITER_OFF: u8 = 0x8E;
const PROMPT_LEVEL: u8 = 0x07;

pub fn read_key(k: u8) -> Vec<u8> {
    vec![0x01, 0x09, k, 0x00]
}

pub fn write_key(k: u8, v: u8) -> Vec<u8> {
    vec![0x00, 0x09, k, 0x00, v]
}

pub fn read_param(p: u8) -> Vec<u8> {
    vec![0x83, 0x2C, p, 0x00]
}

pub fn write_param(p: u8, v: &[u8]) -> Vec<u8> {
    let mut b = vec![0x82, 0x2C, p, 0x00];
    b.extend_from_slice(v);
    b
}

const BATTERY: [u8; 3] = [0xD6, 0x0C, 0x00];

pub fn reports(body: &[u8]) -> Vec<Vec<u8>> {
    let mut msg = vec![0x05, REQUEST];
    msg.extend_from_slice(&(body.len() as u16).to_le_bytes());
    msg.extend_from_slice(body);
    msg.chunks(FRAGMENT)
        .map(|c| {
            let mut r = vec![REPORT_OUT, c.len() as u8, ROUTE];
            r.extend_from_slice(c);
            r
        })
        .collect()
}

/// The new bytes in an input report; a length of 0 means nothing new.
pub fn fragment(r: &[u8]) -> &[u8] {
    match r {
        [REPORT_IN, n, _, rest @ ..]
            if (*n as usize) <= FRAGMENT && (*n as usize) <= rest.len() =>
        {
            &rest[..*n as usize]
        }
        _ => &[],
    }
}

/// Takes the next whole message off the front of the stream, dropping bytes that cannot start one.
pub fn next_message(stream: &mut Vec<u8>) -> Option<Vec<u8>> {
    loop {
        let start = stream.iter().position(|b| *b == 0x05)?;
        stream.drain(..start);
        if stream.len() < 4 {
            return None;
        }
        let size = 4 + u16::from_le_bytes([stream[2], stream[3]]) as usize;
        if size > 1024 {
            stream.remove(0);
            continue;
        }
        if stream.len() < size {
            return None;
        }
        return Some(stream.drain(..size).collect());
    }
}

fn is(m: &[u8], kind: u8, cmd: [u8; 2]) -> bool {
    m.len() >= 6 && m[1] == kind && m[4..6] == cmd
}

pub fn key_value(m: &[u8], k: u8) -> Option<Option<u8>> {
    (is(m, RESPONSE, [0x01, 0x09]) && m.len() >= 8 && m[6..8] == [k, 0])
        .then(|| (m.len() > 9 && m[8] == 0).then(|| m[9]))
}

pub fn key_written(m: &[u8], k: u8) -> bool {
    is(m, RESPONSE, [0x00, 0x09]) && m.len() >= 8 && m[6..8] == [k, 0]
}

pub fn param_value(m: &[u8], p: u8) -> Option<Option<u8>> {
    (is(m, RESPONSE, [0x83, 0x2C]) && m.len() >= 9 && m[7..9] == [p, 0])
        .then(|| (m.len() > 9 && m[6] == 0).then(|| m[9]))
}

pub fn param_written(m: &[u8], p: u8) -> Option<bool> {
    (is(m, RESPONSE, [0x82, 0x2C]) && m.len() >= 9 && m[7..9] == [p, 0]).then(|| m[6] == 0)
}

/// The battery level, or `Some(None)` when the headset refused the request.
pub fn battery(m: &[u8]) -> Option<Option<u8>> {
    if is(m, RESULT, [0xD6, 0x0C]) && m.len() > 8 {
        return Some(Some(m[8].min(100)));
    }
    (is(m, RESPONSE, [0xD6, 0x0C]) && m.len() > 6 && m[6] != 0).then_some(None)
}

/// Auto-off: an on flag and the seconds, twice.
pub fn auto_off(minutes: i32) -> Vec<u8> {
    let secs = (minutes as u16 * 60).to_le_bytes();
    let on = (minutes > 0) as u8;
    vec![on, 0, secs[0], secs[1], on, 0, secs[0], secs[1]]
}

/// Writing the sidetone switch flips it whatever is sent, so it is written only to change it.
pub fn sidetone(on_now: bool, v: i32, max: u8) -> Vec<Vec<u8>> {
    let on = v > 0;
    let mut out = Vec::new();
    if on_now != on {
        out.push(write_param(PARAM_SIDETONE_ON, &[on as u8]));
    }
    if on {
        out.push(write_key(KEY_SIDETONE, (v as u8).min(max.max(1))));
    }
    out
}

/// The dial reads 0 for all chat to 20 for all game.
pub fn mix(raw: u8) -> (u8, u8) {
    let raw = raw.min(20) as u32;
    let game = (raw * 10).min(100) as u8;
    let chat = ((20 - raw) * 10).min(100) as u8;
    (game, chat)
}

const POLL: Duration = Duration::from_millis(50);
const TIMEOUT: Duration = Duration::from_millis(1250);

/// Writes go once: a late reply may still mean the headset applied it.
fn request<T>(
    dev: &Dev,
    body: &[u8],
    tries: usize,
    timeout: Duration,
    pick: impl Fn(&[u8]) -> Option<T>,
) -> Option<T> {
    for _ in 0..tries {
        for r in reports(body) {
            if !dev.write(&r) {
                return None;
            }
        }
        let end = Instant::now() + timeout;
        let mut stream = Vec::new();
        while Instant::now() < end {
            sleep(POLL);
            let r = dev.get_input_report(REPORT_IN)?;
            stream.extend_from_slice(fragment(&r));
            while let Some(m) = next_message(&mut stream) {
                if let Some(v) = pick(&m) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn key(dev: &Dev, k: u8) -> Option<u8> {
    request(dev, &read_key(k), 2, TIMEOUT, |m| key_value(m, k)).flatten()
}

fn param(dev: &Dev, p: u8) -> Option<u8> {
    request(dev, &read_param(p), 2, TIMEOUT, |m| param_value(m, p)).flatten()
}

/// `None` when the headset did not answer: it is off or out of range.
fn level(dev: &Dev) -> Option<Option<u8>> {
    request(dev, &BATTERY, 1, TIMEOUT, battery)
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    // With no headset linked the dongle names itself a dongle.
    if f.coll.product.to_lowercase().ends_with("dongle") {
        return Some(Reading {
            status: Some(Status::default()),
            values: Vec::new(),
        });
    }
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let Some(Some(pct)) = level(&dev) else {
        return Some(Reading {
            status: Some(Status::default()),
            values: Vec::new(),
        });
    };
    let status = Status {
        connected: true,
        // The level reads 0 for a while after power-on.
        battery: (pct > 0).then_some(pct),
        charging: false,
        chatmix: param(&dev, PARAM_MIX).map(mix),
        values: Vec::new(),
    };
    let mut values = Vec::new();
    if settings {
        let mut keys = vec![KEY_SIDETONE, KEY_PRESET, KEY_GATE, KEY_PROMPTS];
        if MAXWELL.contains(&f.model.pid) {
            keys.push(KEY_LIMITER);
        }
        let mut got = Vec::new();
        for k in keys {
            // A silent headset would hold the shared HID turn for every read left.
            let Some(v) = request(&dev, &read_key(k), 2, TIMEOUT, |m| key_value(m, k)) else {
                break;
            };
            got.extend(v.map(|v| (k, v)));
        }
        let on = (!got.is_empty())
            .then(|| param(&dev, PARAM_SIDETONE_ON))
            .flatten();
        values = settings_of(&got, on);
    }
    Some(Reading {
        status: Some(status),
        values,
    })
}

/// Settings from key reads, with the sidetone switch.
pub fn settings_of(keys: &[(u8, u8)], sidetone_on: Option<u8>) -> Vec<(Control, i32)> {
    let mut out = Vec::new();
    for &(k, v) in keys {
        match (k, v) {
            (KEY_SIDETONE, l) if l <= SIDETONE_MAX => {
                if let Some(on) = sidetone_on {
                    out.push((Control::Sidetone, if on == 0 { 0 } else { l as i32 }));
                }
            }
            (KEY_PRESET, p) if (1..=10).contains(&p) => out.push((Control::EqPreset, p as i32)),
            (KEY_GATE, g) if g <= 2 => out.push((Control::NoiseGate, g as i32)),
            (KEY_PROMPTS, p) if p <= 15 => out.push((Control::VoicePrompts, (p > 0) as i32)),
            (KEY_LIMITER, LIMITER_ON) => out.push((Control::VolumeLimiter, 1)),
            (KEY_LIMITER, LIMITER_OFF) => out.push((Control::VolumeLimiter, 0)),
            _ => {}
        }
    }
    out
}

fn write(dev: &Dev, body: &[u8]) -> Result<(), String> {
    let took = match body[..2] {
        [0x00, 0x09] => request(dev, body, 1, TIMEOUT, |m| {
            key_written(m, body[2]).then_some(true)
        }),
        _ => request(dev, body, 1, TIMEOUT, |m| param_written(m, body[2])),
    };
    match took {
        Some(true) => Ok(()),
        Some(false) => Err("the headset refused the setting".into()),
        None => Err("the headset did not answer".into()),
    }
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    let shape = controls(&f.model).into_iter().find(|(k, _)| *k == c);
    match shape {
        None => return Err(format!("{} has no {} setting", f.model.name, c.id())),
        Some((_, s)) if !s.accepts(v) => {
            return Err(format!("{v} is out of range for {}", c.id()));
        }
        _ => {}
    }
    let dev = Dev::open(&f.coll).ok_or("the headset's dongle could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    let bodies = match c {
        Control::Sidetone => {
            let on = param(&dev, PARAM_SIDETONE_ON)
                .ok_or("the headset did not say if sidetone is on")?;
            let max = key(&dev, KEY_SIDETONE_MAX)
                .filter(|m| *m > 0)
                .unwrap_or(SIDETONE_MAX);
            sidetone(on != 0, v, max)
        }
        Control::AutoOff => vec![write_param(PARAM_OFF, &auto_off(v))],
        Control::EqPreset => vec![write_key(KEY_PRESET, v as u8)],
        Control::NoiseGate => vec![write_key(KEY_GATE, v as u8)],
        Control::VoicePrompts => vec![write_key(
            KEY_PROMPTS,
            if v == 1 { PROMPT_LEVEL } else { 0 },
        )],
        Control::VolumeLimiter => vec![write_key(
            KEY_LIMITER,
            if v == 1 { LIMITER_ON } else { LIMITER_OFF },
        )],
        _ => return Err(format!("{} has no {} setting", f.model.name, c.id())),
    };
    for b in bodies {
        write(&dev, &b)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(body: &[u8]) -> Vec<u8> {
        let r = reports(body);
        assert_eq!(r.len(), 1);
        r.into_iter().next().unwrap()
    }

    #[test]
    fn models() {
        let m = crate::proto::any_model(VID, 0x4B19).unwrap();
        assert_eq!(
            (m.brand(), m.name, m.family),
            ("Audeze", "Maxwell", Family::Audeze)
        );
        assert_eq!(m.collection(), (0, Some((0xFF13, 0x0001))));
        let m2 = crate::proto::any_model(VID, 0x4B28).unwrap();
        assert_eq!((m2.name, m2.collection().0), ("Maxwell 2", 5));
        assert!(crate::proto::any_model(VID, 0x4B1A).is_none());
        let has = |m: &Model, c| m.controls().iter().any(|(k, _)| *k == c);
        assert!(has(&m, Control::VolumeLimiter) && !has(&m2, Control::VolumeLimiter));
        assert!(has(&m2, Control::VoicePrompts) && !has(&m2, Control::MicVolume));
        assert!(m.set(Control::EqPreset, 1).is_err());
        assert_eq!(m.save(Some(Control::Sidetone)), crate::Save::None);
    }

    #[test]
    fn request_reports() {
        assert_eq!(
            one(&write_key(KEY_PRESET, 3)),
            [
                0x06, 0x09, 0x80, 0x05, 0x5A, 0x05, 0x00, 0x00, 0x09, 0x00, 0x00, 0x03
            ]
        );
        assert_eq!(
            one(&write_key(KEY_LIMITER, LIMITER_ON)),
            [
                0x06, 0x09, 0x80, 0x05, 0x5A, 0x05, 0x00, 0x00, 0x09, 0x28, 0x00, 0x88
            ]
        );
        assert_eq!(
            one(&write_key(KEY_PROMPTS, PROMPT_LEVEL)),
            [
                0x06, 0x09, 0x80, 0x05, 0x5A, 0x05, 0x00, 0x00, 0x09, 0x20, 0x00, 0x07
            ]
        );
        assert_eq!(
            one(&write_key(KEY_GATE, 2)),
            [
                0x06, 0x09, 0x80, 0x05, 0x5A, 0x05, 0x00, 0x00, 0x09, 0x24, 0x00, 0x02
            ]
        );
        assert_eq!(
            one(&write_param(PARAM_OFF, &auto_off(30))),
            [
                0x06, 0x10, 0x80, 0x05, 0x5A, 0x0C, 0x00, 0x82, 0x2C, 0x01, 0x00, 0x01, 0x00, 0x08,
                0x07, 0x01, 0x00, 0x08, 0x07
            ]
        );
        assert_eq!(auto_off(0), [0; 8]);
        assert_eq!(auto_off(360), [1, 0, 0x60, 0x54, 1, 0, 0x60, 0x54]);
        assert_eq!(
            one(&read_key(KEY_SIDETONE)),
            [
                0x06, 0x08, 0x80, 0x05, 0x5A, 0x04, 0x00, 0x01, 0x09, 0x2C, 0x00
            ]
        );
        assert_eq!(
            one(&read_param(PARAM_MIX)),
            [
                0x06, 0x08, 0x80, 0x05, 0x5A, 0x04, 0x00, 0x83, 0x2C, 0x0B, 0x00
            ]
        );
        assert_eq!(
            one(&BATTERY),
            [0x06, 0x07, 0x80, 0x05, 0x5A, 0x03, 0x00, 0xD6, 0x0C, 0x00]
        );
        let long = reports(&[0xAA; 70]);
        assert_eq!(long.len(), 2);
        assert_eq!((long[0][1], long[1][1]), (59, 15));
    }

    #[test]
    fn sidetone_plan() {
        let on = write_param(PARAM_SIDETONE_ON, &[1]);
        let off = write_param(PARAM_SIDETONE_ON, &[0]);
        assert_eq!(
            sidetone(false, 12, 31),
            vec![on, write_key(KEY_SIDETONE, 12)]
        );
        assert_eq!(sidetone(true, 12, 31), vec![write_key(KEY_SIDETONE, 12)]);
        assert_eq!(sidetone(true, 0, 31), vec![off]);
        assert!(sidetone(false, 0, 31).is_empty());
        assert_eq!(sidetone(true, 31, 20), vec![write_key(KEY_SIDETONE, 20)]);
    }

    #[test]
    fn stream_and_replies() {
        let mut r = vec![0u8; 62];
        r[0] = 0x07;
        r[1] = 10;
        r[2] = 0x80;
        r[3..13].copy_from_slice(&[0x05, 0x5B, 0x06, 0x00, 0x01, 0x09, 0x2C, 0x00, 0x00, 0x0E]);
        // Stale bytes past the length must be ignored.
        r[13..18].copy_from_slice(&[0x05, 0x5B, 0x06, 0x00, 0x83]);
        let mut s = fragment(&r).to_vec();
        let m = next_message(&mut s).unwrap();
        assert_eq!(key_value(&m, KEY_SIDETONE), Some(Some(14)));
        assert_eq!(key_value(&m, KEY_PRESET), None);
        assert!(next_message(&mut s).is_none());
        r[1] = 0;
        assert!(fragment(&r).is_empty());
        r[1] = 60;
        assert!(fragment(&r).is_empty());

        let mut s = vec![0xFF, 0x05, 0x5B, 0x06];
        assert!(next_message(&mut s).is_none());
        s.extend_from_slice(&[0x00, 0x83, 0x2C, 0x00, 0x07, 0x00, 0x01]);
        let m = next_message(&mut s).unwrap();
        assert_eq!(param_value(&m, PARAM_SIDETONE_ON), Some(Some(1)));
        assert_eq!(param_value(&m, PARAM_MIX), None);
        assert_eq!(
            param_value(
                &[0x05, 0x5B, 0x06, 0x00, 0x83, 0x2C, 0x01, 0x07, 0x00, 0x01],
                0x07
            ),
            Some(None)
        );
        assert_eq!(
            key_value(
                &[0x05, 0x5B, 0x06, 0x00, 0x01, 0x09, 0x2C, 0x00, 0x03, 0x0E],
                0x2C
            ),
            Some(None)
        );
        assert!(key_written(
            &[0x05, 0x5B, 0x05, 0x00, 0x00, 0x09, 0x28, 0x00, 0x00],
            0x28
        ));
        assert!(!key_written(
            &[0x05, 0x5B, 0x05, 0x00, 0x00, 0x09, 0x24, 0x00, 0x00],
            0x28
        ));
        assert_eq!(
            param_written(
                &[0x05, 0x5B, 0x05, 0x00, 0x82, 0x2C, 0x00, 0x01, 0x00],
                0x01
            ),
            Some(true)
        );
        assert_eq!(
            param_written(
                &[0x05, 0x5B, 0x05, 0x00, 0x82, 0x2C, 0x02, 0x01, 0x00],
                0x01
            ),
            Some(false)
        );
        assert_eq!(
            battery(&[0x05, 0x5D, 0x05, 0x00, 0xD6, 0x0C, 0x00, 0x00, 0x47]),
            Some(Some(71))
        );
        assert_eq!(battery(&[0x05, 0x5B, 0x03, 0x00, 0xD6, 0x0C, 0x00]), None);
        assert_eq!(
            battery(&[0x05, 0x5B, 0x03, 0x00, 0xD6, 0x0C, 0x01]),
            Some(None)
        );
    }

    #[test]
    fn read_back() {
        let keys = [
            (KEY_SIDETONE, 12),
            (KEY_PRESET, 7),
            (KEY_GATE, 2),
            (KEY_PROMPTS, 7),
            (KEY_LIMITER, 0x8E),
        ];
        assert_eq!(
            settings_of(&keys, Some(1)),
            vec![
                (Control::Sidetone, 12),
                (Control::EqPreset, 7),
                (Control::NoiseGate, 2),
                (Control::VoicePrompts, 1),
                (Control::VolumeLimiter, 0)
            ]
        );
        assert_eq!(
            settings_of(&keys[..1], Some(0)),
            vec![(Control::Sidetone, 0)]
        );
        assert!(settings_of(&keys[..1], None).is_empty());
        assert!(settings_of(&[(KEY_PRESET, 11), (KEY_GATE, 3), (KEY_LIMITER, 1)], None).is_empty());
    }

    #[test]
    fn chatmix() {
        assert_eq!(mix(10), (100, 100));
        assert_eq!(mix(0), (0, 100));
        assert_eq!(mix(20), (100, 0));
        assert_eq!(mix(15), (100, 50));
        assert_eq!(mix(200), (100, 0));
    }
}
