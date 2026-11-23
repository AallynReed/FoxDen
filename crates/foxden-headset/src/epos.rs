//! EPOS gaming headsets that take settings over HID: the H3PRO Hybrid, by dongle or cable.

use std::time::Duration;

use foxden_battery::hid::Dev;

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x1395;
const DONGLE: u16 = 0x0303;
const CABLE: u16 = 0x0101;

pub fn model(pid: u16) -> Option<Model> {
    matches!(pid, DONGLE | CABLE).then_some(Model {
        vid: VID,
        pid,
        name: "H3PRO Hybrid",
        family: Family::Epos,
        sidetone: true,
        sidetone_max: 0,
        mic: false,
        anc: false,
        wireless: pid == DONGLE,
    })
}

const REPORT: u8 = 0x01;
const REQUEST: u16 = 0x0004;
/// The dongle or a cabled headset, and the headset behind the dongle. Replies come from one less.
const PLUGGED: u16 = 0x0001;
const HEADSET: u16 = 0x0005;

const CONNECTED: (u16, u16) = (0x01, 0x09);
const BATTERY: (u16, u16) = (0x12, 0x03);
const SIDETONE_SET: (u16, u16) = (0x0A, 0x0B);
const SIDETONE_GET: (u16, u16) = (0x0A, 0x0C);
const MIC_NR_SET: (u16, u16) = (0x0A, 0x11);
const MIC_NR_GET: (u16, u16) = (0x0A, 0x12);

pub fn controls() -> Vec<(Control, Shape)> {
    vec![
        (Control::Sidetone, Shape::Steps { min: 0, max: 255 }),
        (Control::MicNoiseFilter, Shape::Switch),
    ]
}

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0xFFu8;
    for b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// `01 EB FE`, then little-endian words: version, type, target, payload length, CRC, payload.
fn frame(target: u16, (group, cmd): (u16, u16), value: Option<u16>) -> Vec<u8> {
    let payload: Vec<u8> = [group, cmd]
        .into_iter()
        .chain(value)
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut f = vec![REPORT, 0xEB, 0xFE];
    for w in [
        1,
        REQUEST,
        target,
        payload.len() as u16,
        crc8(&payload) as u16,
    ] {
        f.extend(w.to_le_bytes());
    }
    f.extend(payload);
    f
}

#[derive(Debug, PartialEq)]
struct Reply {
    kind: u16,
    target: u16,
    what: (u16, u16),
    values: Vec<u16>,
}

fn parse(r: &[u8]) -> Option<Reply> {
    if r.len() < 13 || r[..3] != [REPORT, 0xEB, 0xFE] {
        return None;
    }
    let word = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]);
    let len = word(9) as usize;
    if len < 4
        || !len.is_multiple_of(2)
        || r.len() < 13 + len
        || word(11) != crc8(&r[13..13 + len]) as u16
    {
        return None;
    }
    Some(Reply {
        kind: word(5),
        target: word(7),
        what: (word(13), word(15)),
        values: (17..13 + len).step_by(2).map(word).collect(),
    })
}

#[derive(Debug, PartialEq)]
enum Answer {
    Values(Vec<u16>),
    /// The dongle sent the request back: the headset is off.
    Off,
}

fn answer(r: &[u8], target: u16, what: (u16, u16)) -> Option<Answer> {
    let f = parse(r).filter(|f| f.kind == REQUEST && f.what == what)?;
    if target == HEADSET && f.target == PLUGGED {
        Some(Answer::Off)
    } else {
        (f.target == target - 1).then_some(Answer::Values(f.values))
    }
}

const WAIT: Duration = Duration::from_millis(1000);

fn ask(dev: &Dev, target: u16, what: (u16, u16), value: Option<u16>) -> Option<Answer> {
    dev.flush();
    if !dev.write(&frame(target, what, value)) {
        return None;
    }
    dev.read_until(WAIT, |r| answer(r, target, what))
}

fn one(a: Option<Answer>, max: u16) -> Option<i32> {
    match a {
        Some(Answer::Values(v)) if v.len() == 1 && v[0] <= max => Some(v[0] as i32),
        _ => None,
    }
}

fn target(m: &Model) -> u16 {
    if m.wireless { HEADSET } else { PLUGGED }
}

/// `None` when nothing answered, `Some(false)` when the dongle has no headset.
fn headset_on(dev: &Dev, m: &Model) -> Option<bool> {
    if !m.wireless {
        return Some(true);
    }
    one(ask(dev, PLUGGED, CONNECTED, None), 1).map(|v| v == 1)
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let t = target(&f.model);
    let on = match headset_on(&dev, &f.model) {
        Some(on) => on,
        None => return Some(Reading::default()),
    };
    let battery = on.then(|| one(ask(&dev, t, BATTERY, None), 100)).flatten();
    if !f.model.wireless && battery.is_none() {
        return Some(Reading::default());
    }
    let mut values = Vec::new();
    if on && settings {
        if let Some(v) = one(ask(&dev, t, SIDETONE_GET, None), 255) {
            values.push((Control::Sidetone, v));
        }
        if let Some(v) = one(ask(&dev, t, MIC_NR_GET, None), 1) {
            values.push((Control::MicNoiseFilter, v));
        }
    }
    Some(Reading {
        status: Some(Status {
            connected: on,
            battery: battery.map(|b| b as u8),
            ..Status::default()
        }),
        values,
    })
}

fn set_frame(m: &Model, c: Control, v: i32) -> Result<Vec<u8>, String> {
    if !controls().iter().any(|(k, s)| *k == c && s.accepts(v)) {
        return Err(format!("{} has no {} setting of {v}", m.name, c.id()));
    }
    let what = if c == Control::Sidetone {
        SIDETONE_SET
    } else {
        MIC_NR_SET
    };
    Ok(frame(target(m), what, Some(v as u16)))
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    let req = set_frame(&f.model, c, v)?;
    let what = (
        u16::from_le_bytes([req[13], req[14]]),
        u16::from_le_bytes([req[15], req[16]]),
    );
    let dev = Dev::open(&f.coll).ok_or("the headset could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    if headset_on(&dev, &f.model) != Some(true) {
        return Err("the headset is switched off".into());
    }
    let t = target(&f.model);
    dev.flush();
    if !dev.write(&req) {
        return Err("the headset did not take the setting".into());
    }
    match one(dev.read_until(WAIT, |r| answer(r, t, what)), u16::MAX) {
        Some(0) => Ok(()),
        Some(_) => Err("the headset refused the setting".into()),
        None => Err("the headset did not answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(v: &[u8]) -> Vec<u8> {
        v.to_vec()
    }

    #[test]
    fn frames_match_captures() {
        assert_eq!(
            frame(PLUGGED, CONNECTED, None),
            bytes(&[
                0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0x01, 0, 0x04, 0, 0x7A, 0, 0x01, 0, 0x09, 0
            ])
        );
        assert_eq!(
            frame(HEADSET, BATTERY, None),
            bytes(&[
                0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0x05, 0, 0x04, 0, 0xA5, 0, 0x12, 0, 0x03, 0
            ])
        );
        let cable = model(CABLE).unwrap();
        assert_eq!(
            set_frame(&cable, Control::Sidetone, 255).unwrap(),
            bytes(&[
                0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0x01, 0, 0x06, 0, 0xEF, 0, 0x0A, 0, 0x0B, 0,
                0xFF, 0
            ])
        );
        assert_eq!(
            set_frame(&cable, Control::Sidetone, 0).unwrap(),
            bytes(&[
                0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0x01, 0, 0x06, 0, 0x38, 0, 0x0A, 0, 0x0B, 0,
                0x00, 0
            ])
        );
        assert_eq!(
            set_frame(&cable, Control::MicNoiseFilter, 1).unwrap(),
            bytes(&[
                0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0x01, 0, 0x06, 0, 0xD6, 0, 0x0A, 0, 0x11, 0,
                0x01, 0
            ])
        );
        assert_eq!(
            set_frame(&cable, Control::MicNoiseFilter, 0).unwrap(),
            bytes(&[
                0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0x01, 0, 0x06, 0, 0xC3, 0, 0x0A, 0, 0x11, 0,
                0x00, 0
            ])
        );
        let dongle = model(DONGLE).unwrap();
        assert_eq!(set_frame(&dongle, Control::Sidetone, 10).unwrap()[7], 0x05);
        assert!(set_frame(&dongle, Control::Sidetone, 256).is_err());
        assert!(set_frame(&dongle, Control::MicNoiseFilter, 2).is_err());
        assert!(set_frame(&dongle, Control::Anc, 1).is_err());
        assert!(model(0x009A).is_none());
    }

    #[test]
    fn replies_match_captures() {
        let battery = [
            0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0, 0, 0x06, 0, 0x5F, 0, 0x12, 0, 0x03, 0, 0x28, 0,
        ];
        assert_eq!(one(answer(&battery, PLUGGED, BATTERY), 100), Some(40));
        assert_eq!(answer(&battery, PLUGGED, SIDETONE_GET), None);
        let mut bad = battery;
        bad[17] = 0x29;
        assert_eq!(parse(&bad), None);
        assert_eq!(parse(&battery[..17]), None);
        assert_eq!(parse(&[0x05, 0x01]), None);
        let sidetone = [
            0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0, 0, 0x06, 0, 0xAB, 0, 0x0A, 0, 0x0C, 0, 0xAD, 0,
        ];
        assert_eq!(
            one(answer(&sidetone, PLUGGED, SIDETONE_GET), 255),
            Some(0xAD)
        );
        let ok = [
            0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0, 0, 0x06, 0, 0x38, 0, 0x0A, 0, 0x0B, 0, 0, 0,
        ];
        assert_eq!(one(answer(&ok, PLUGGED, SIDETONE_SET), u16::MAX), Some(0));
        let connected = [
            0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0, 0, 0x06, 0, 0x35, 0, 0x01, 0, 0x09, 0, 0x01, 0,
        ];
        assert_eq!(one(answer(&connected, PLUGGED, CONNECTED), 1), Some(1));
        let gone = [
            0x01, 0xEB, 0xFE, 0x01, 0, 0x04, 0, 0, 0, 0x06, 0, 0x20, 0, 0x01, 0, 0x09, 0, 0, 0,
        ];
        assert_eq!(one(answer(&gone, PLUGGED, CONNECTED), 1), Some(0));
        let event = [
            0x01, 0xEB, 0xFE, 0x01, 0, 0x10, 0x01, 0, 0, 0x08, 0, 0xF4, 0, 0x01, 0, 0xF0, 0, 0x01,
            0, 0x01, 0,
        ];
        assert!(parse(&event).is_some());
        assert_eq!(answer(&event, PLUGGED, (0x01, 0xF0)), None);
        let returned = frame(PLUGGED, BATTERY, None);
        assert_eq!(answer(&returned, HEADSET, BATTERY), Some(Answer::Off));
        let from_headset = frame(HEADSET - 1, BATTERY, Some(60));
        assert_eq!(one(answer(&from_headset, HEADSET, BATTERY), 100), Some(60));
    }
}
