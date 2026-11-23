//! ASTRO A50 base stations: Gen 4 replies carry a status byte, Gen 5 frames carry a marker.

use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use foxden_battery::hid::{Coll, Dev};

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x9886;
pub const GEN5_PID: u16 = 0x0B1C;
const WAIT: Duration = Duration::from_secs(1);

const fn row(vid: u16, pid: u16, name: &'static str, family: Family) -> Model {
    Model {
        vid,
        pid,
        name,
        family,
        sidetone: true,
        sidetone_max: 0,
        mic: false,
        anc: false,
        wireless: true,
    }
}

pub fn model(vid: u16, pid: u16) -> Option<Model> {
    match (vid, pid) {
        (VID, 0x002C) => Some(row(VID, pid, "A50 Gen 4", Family::AstroGen4)),
        (0x046D, GEN5_PID) => Some(row(vid, pid, "A50 Gen 5", Family::AstroGen5)),
        _ => None,
    }
}

const GEN4_GATE: [(i32, &str); 4] = [
    (0, "Streaming"),
    (1, "Night"),
    (2, "Home"),
    (3, "Tournament"),
];
const GEN5_GATE: [(i32, &str); 3] = [(0, "Off"), (1, "Night"), (4, "Tournament")];

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let choice = |o: &[(i32, &'static str)]| Shape::Choice {
        options: o.to_vec(),
    };
    match m.family {
        Family::AstroGen4 => vec![
            (Control::Sidetone, Shape::Steps { min: 0, max: 100 }),
            (Control::MicVolume, Shape::Steps { min: 0, max: 100 }),
            (Control::NoiseGate, choice(&GEN4_GATE)),
            (
                Control::EqPreset,
                choice(&[(1, "Preset 1"), (2, "Preset 2"), (3, "Preset 3")]),
            ),
        ],
        Family::AstroGen5 => vec![
            (Control::Sidetone, Shape::Switch),
            (Control::NoiseGate, choice(&GEN5_GATE)),
        ],
        _ => Vec::new(),
    }
}

fn checked(m: &Model, c: Control, v: i32) -> Result<(), String> {
    match controls(m).into_iter().find(|(k, _)| *k == c) {
        None => Err(format!("{} has no {} setting", m.name, c.id())),
        Some((_, s)) if !s.accepts(v) => Err(format!("{v} is out of range for {}", c.id())),
        Some(_) => Ok(()),
    }
}

/// The battery driver's vendor collection test.
pub fn gen4_fits(c: &Coll) -> bool {
    matches!(c.page, 0xFF31 | 0xFF32 | 0xFF00) && c.in_len >= 64 && c.out_len >= 64
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    match f.model.family {
        Family::AstroGen4 => gen4::read(f, settings),
        _ => gen5::read(f),
    }
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    checked(&f.model, c, v)?;
    match f.model.family {
        Family::AstroGen4 => gen4::set(f, c, v),
        _ => gen5::set(f, c, v),
    }
}

/// Game and chat sides, 0..=100, from a balance where 0 is all chat and `max` all game.
fn sides(raw: u32, max: u32) -> (u8, u8) {
    let raw = raw.min(max);
    let game = (raw * 200 / max).min(100) as u8;
    let chat = ((max - raw) * 200 / max).min(100) as u8;
    (game, chat)
}

mod gen4 {
    use super::*;

    const LINK: u8 = 0x54;
    const SET_SLIDER: u8 = 0x62;
    const SET_GATE: u8 = 0x64;
    const SET_PRESET: u8 = 0x67;
    const GET_SLIDER: u8 = 0x68;
    const GET_GATE: u8 = 0x6A;
    const GET_PRESET: u8 = 0x6C;
    const BALANCE: u8 = 0x72;
    const BATTERY: u8 = 0x7C;
    const MIC: u8 = 0x04;
    const SIDETONE: u8 = 0x05;

    /// `02 cmd`, or `00 02 cmd` on a collection without report ids.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Framing {
        Numbered,
        Bare,
    }

    static FRAMING: Mutex<Option<Framing>> = Mutex::new(None);

    pub fn request(f: Framing, cmd: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        if f == Framing::Bare {
            v.push(0);
        }
        v.extend_from_slice(&[0x02, cmd]);
        if !payload.is_empty() {
            v.push(payload.len() as u8);
            v.extend_from_slice(payload);
        }
        v
    }

    #[derive(Debug, PartialEq)]
    pub enum Reply {
        /// Everything after the length byte, which the firmware sometimes gets wrong.
        Data(Vec<u8>),
        Accepted,
        Refused(String),
    }

    pub fn reply(r: &[u8]) -> Option<Reply> {
        if r.len() < 3 || r[0] != 0x02 {
            return None;
        }
        match r[1] {
            0x02 => Some(Reply::Data(r[3..].to_vec())),
            0x00 => Some(Reply::Accepted),
            0x01 => {
                let name: String = r
                    .get(7..)
                    .unwrap_or_default()
                    .iter()
                    .take_while(|b| **b != 0)
                    .map(|b| {
                        if (0x20..0x7F).contains(b) {
                            *b as char
                        } else {
                            '?'
                        }
                    })
                    .collect();
                Some(Reply::Refused(name))
            }
            _ => None,
        }
    }

    pub fn write(c: Control, v: i32) -> Option<(u8, Vec<u8>)> {
        let b = v as u8;
        Some(match c {
            Control::Sidetone => (SET_SLIDER, vec![SIDETONE, b]),
            Control::MicVolume => (SET_SLIDER, vec![MIC, b]),
            Control::NoiseGate => (SET_GATE, vec![b]),
            Control::EqPreset => (SET_PRESET, vec![b]),
            _ => return None,
        })
    }

    /// Whether a data reply to [`write`] echoes what was sent.
    pub fn echoed(c: Control, v: i32, d: &[u8]) -> bool {
        let b = v as u8;
        match c {
            Control::Sidetone => d.starts_with(&[SET_SLIDER, SIDETONE]),
            Control::MicVolume => d.starts_with(&[SET_SLIDER, MIC]),
            Control::NoiseGate => d.first() == Some(&b),
            Control::EqPreset => d.starts_with(&[SET_PRESET, b]),
            _ => false,
        }
    }

    pub fn slider(d: &[u8], id: u8) -> Option<i32> {
        (d.len() >= 3 && d[0] == GET_SLIDER && d[1] == id && d[2] <= 100).then(|| d[2] as i32)
    }

    pub fn gate(d: &[u8]) -> Option<i32> {
        (d.len() >= 2 && d[0] == GET_GATE && d[1] <= 3).then(|| d[1] as i32)
    }

    pub fn preset(d: &[u8]) -> Option<i32> {
        d.first()
            .filter(|p| (1..=3).contains(*p))
            .map(|p| *p as i32)
    }

    pub fn battery(d: &[u8]) -> Option<(u8, bool)> {
        d.first()
            .filter(|b| *b & 0x7F <= 100)
            .map(|b| (b & 0x7F, b & 0x80 != 0))
    }

    /// The balance moves on a 5-step grid, so its middle reads 125 or 130.
    pub fn balance(d: &[u8]) -> Option<(u8, u8)> {
        let raw = *d.first()?;
        Some(if (125..=130).contains(&raw) {
            (100, 100)
        } else {
            sides(raw as u32, 255)
        })
    }

    fn ask(dev: &Dev, f: Framing, cmd: u8, payload: &[u8]) -> Option<Reply> {
        dev.flush();
        if !dev.write(&request(f, cmd, payload)) {
            return None;
        }
        dev.read_until(WAIT, reply)
    }

    fn data(dev: &Dev, f: Framing, cmd: u8, payload: &[u8]) -> Option<Vec<u8>> {
        match ask(dev, f, cmd, payload)? {
            Reply::Data(d) => Some(d),
            _ => None,
        }
    }

    /// The link byte (bit 0 docked, bit 1 linked), finding the framing on the first answer.
    fn link(dev: &Dev) -> Option<(Framing, u8)> {
        let known = *FRAMING.lock().unwrap_or_else(|e| e.into_inner());
        let order = match known {
            Some(Framing::Bare) => [Framing::Bare, Framing::Numbered],
            _ => [Framing::Numbered, Framing::Bare],
        };
        for f in order {
            if let Some(d) = data(dev, f, LINK, &[]) {
                *FRAMING.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
                return Some((f, d.first().copied().unwrap_or(0)));
            }
        }
        None
    }

    pub fn read(f: &Found, settings: bool) -> Option<Reading> {
        let dev = Dev::open(&f.coll)?;
        let _turn = foxden_core::hid_turn::take();
        let Some((fr, link)) = link(&dev) else {
            return Some(Reading::default());
        };
        let mut status = Status {
            connected: link & 0x03 != 0,
            ..Status::default()
        };
        let mut values = Vec::new();
        if status.connected {
            if let Some((pct, charging)) = data(&dev, fr, BATTERY, &[]).and_then(|d| battery(&d)) {
                status.battery = Some(pct);
                status.charging = charging || (link & 0x01 != 0 && pct < 100);
            }
            if link & 0x02 != 0 {
                status.chatmix = data(&dev, fr, BALANCE, &[]).and_then(|d| balance(&d));
            }
        }
        if status.connected && settings {
            for (c, id) in [(Control::Sidetone, SIDETONE), (Control::MicVolume, MIC)] {
                if let Some(v) = data(&dev, fr, GET_SLIDER, &[id]).and_then(|d| slider(&d, id)) {
                    values.push((c, v));
                }
            }
            if let Some(v) = data(&dev, fr, GET_GATE, &[]).and_then(|d| gate(&d)) {
                values.push((Control::NoiseGate, v));
            }
            if let Some(v) = data(&dev, fr, GET_PRESET, &[]).and_then(|d| preset(&d)) {
                values.push((Control::EqPreset, v));
            }
        }
        Some(Reading {
            status: Some(status),
            values,
        })
    }

    pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
        let (cmd, payload) = write(c, v).ok_or("this setting is not on the base station")?;
        let dev = Dev::open(&f.coll).ok_or("the base station could not be opened")?;
        let _turn = foxden_core::hid_turn::take();
        let (fr, _) = link(&dev).ok_or("the base station did not answer")?;
        match ask(&dev, fr, cmd, &payload) {
            Some(Reply::Accepted) => {}
            Some(Reply::Data(d)) if echoed(c, v, &d) => {}
            Some(Reply::Data(_)) => return Err("the base station answered something else".into()),
            Some(Reply::Refused(e)) => return Err(format!("the base station refused it ({e})")),
            None => return Err("the base station did not answer".into()),
        }
        if c == Control::EqPreset {
            // The preset reads back late.
            for _ in 0..8 {
                sleep(Duration::from_millis(200));
                if data(&dev, fr, GET_PRESET, &[]).and_then(|d| preset(&d)) == Some(v) {
                    return Ok(());
                }
            }
            return Err("the base station did not confirm the preset".into());
        }
        Ok(())
    }
}

mod gen5 {
    use super::*;

    const BATTERY: u8 = 0x06;
    const SIDETONE: u8 = 0x09;
    const MIX: u8 = 0x0A;
    const GATE: u8 = 0x14;

    pub fn frame(cmd: u8, handle: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0x02, 0x0C, 3 + payload.len() as u8, 0x00, cmd, handle];
        v.extend_from_slice(payload);
        v
    }

    /// A reply or push for `cmd` that carries a value.
    pub fn answer(r: &[u8], cmd: u8) -> bool {
        r.len() >= 7 && r[0] == 0x02 && r[1] == 0x0C && r[2] > 3 && r[3] == 0 && r[4] == cmd
    }

    pub fn battery(r: &[u8]) -> Option<(u8, bool)> {
        (answer(r, BATTERY) && r.len() >= 9 && r[6] <= 100).then(|| (r[6], r[8] != 0))
    }

    /// The dial reads 0 for all chat to 12 for all game.
    pub fn mix(r: &[u8]) -> Option<(u8, u8)> {
        answer(r, MIX).then(|| sides(r[6] as u32, 12))
    }

    pub fn write(c: Control, v: i32) -> Option<Vec<u8>> {
        match c {
            Control::Sidetone => Some(frame(SIDETONE, 0x1C, &[0x01, 0xFF, v as u8 * 6])),
            Control::NoiseGate => Some(frame(GATE, 0x2D, &[v as u8])),
            _ => None,
        }
    }

    fn ask<T>(dev: &Dev, cmd: u8, pick: impl Fn(&[u8]) -> Option<T>) -> Option<T> {
        dev.flush();
        if !dev.write(&frame(cmd, 0x0C, &[])) {
            return None;
        }
        dev.read_until(WAIT, |r| answer(r, cmd).then(|| pick(r)).flatten())
    }

    pub fn read(f: &Found) -> Option<Reading> {
        let dev = Dev::open(&f.coll)?;
        let _turn = foxden_core::hid_turn::take();
        let status = ask(&dev, BATTERY, battery).map(|(pct, charging)| Status {
            connected: true,
            battery: Some(pct),
            charging,
            chatmix: ask(&dev, MIX, mix),
            values: Vec::new(),
        });
        Some(Reading {
            status,
            values: Vec::new(),
        })
    }

    pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
        let frame = write(c, v).ok_or("this setting is not on the base station")?;
        let dev = Dev::open(&f.coll).ok_or("the base station could not be opened")?;
        let _turn = foxden_core::hid_turn::take();
        if dev.write(&frame) {
            Ok(())
        } else {
            Err("the base station did not take the setting".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::gen4::{Framing, Reply};
    use super::*;

    #[test]
    fn models_and_controls() {
        let g4 = crate::proto::any_model(VID, 0x002C).unwrap();
        assert_eq!(
            (g4.family, g4.brand(), g4.name),
            (Family::AstroGen4, "ASTRO", "A50 Gen 4")
        );
        assert_eq!(g4.collection(), (6, Some((0xFF32, 0x0074))));
        let g5 = crate::proto::any_model(0x046D, GEN5_PID).unwrap();
        assert_eq!(g5.family, Family::AstroGen5);
        assert!(crate::proto::any_model(VID, 0x0015).is_none());
        assert!(crate::proto::any_model(VID, 0x002D).is_none());
        assert!(crate::proto::any_model(0x046D, 0x0B0B).is_none());
        assert!(g4.set(Control::Sidetone, 10).is_err());
        assert!(!g4.has_eq() && g4.chatmix_switch(true).is_none());
        assert_eq!(g4.save(Some(Control::Sidetone)), crate::Save::None);
        assert!(checked(&g4, Control::Sidetone, 101).is_err());
        assert!(checked(&g4, Control::EqPreset, 0).is_err());
        assert!(checked(&g4, Control::NoiseGate, 4).is_err());
        assert!(checked(&g4, Control::AutoOff, 10).is_err());
        assert!(checked(&g5, Control::NoiseGate, 2).is_err());
        assert!(checked(&g5, Control::NoiseGate, 4).is_ok());
        assert!(checked(&g5, Control::MicVolume, 1).is_err());
    }

    #[test]
    fn gen4_requests() {
        use gen4::request;
        assert_eq!(request(Framing::Numbered, 0x54, &[]), [0x02, 0x54]);
        assert_eq!(request(Framing::Bare, 0x54, &[]), [0x00, 0x02, 0x54]);
        let (cmd, p) = gen4::write(Control::Sidetone, 40).unwrap();
        assert_eq!(
            request(Framing::Numbered, cmd, &p),
            [0x02, 0x62, 0x02, 0x05, 40]
        );
        let (cmd, p) = gen4::write(Control::MicVolume, 100).unwrap();
        assert_eq!(
            request(Framing::Bare, cmd, &p),
            [0x00, 0x02, 0x62, 0x02, 0x04, 100]
        );
        let (cmd, p) = gen4::write(Control::NoiseGate, 3).unwrap();
        assert_eq!(
            request(Framing::Numbered, cmd, &p),
            [0x02, 0x64, 0x01, 0x03]
        );
        let (cmd, p) = gen4::write(Control::EqPreset, 2).unwrap();
        assert_eq!(
            request(Framing::Numbered, cmd, &p),
            [0x02, 0x67, 0x01, 0x02]
        );
        assert_eq!(
            request(Framing::Numbered, 0x68, &[0x05]),
            [0x02, 0x68, 0x01, 0x05]
        );
        assert!(gen4::write(Control::AutoOff, 1).is_none());
    }

    #[test]
    fn gen4_replies() {
        use gen4::{echoed, reply};
        let mut err = vec![0x02, 0x01, 0x20, 0x07, 0, 0, 0];
        err.extend_from_slice(b"HID_ERROR_SLAVE_NO_SLAVE\0junk");
        assert_eq!(
            reply(&err),
            Some(Reply::Refused("HID_ERROR_SLAVE_NO_SLAVE".into()))
        );
        assert_eq!(reply(&[0x02, 0x00, 0x00]), Some(Reply::Accepted));
        assert_eq!(
            reply(&[0x02, 0x02, 0x02, 0x62, 0x05]),
            Some(Reply::Data(vec![0x62, 0x05]))
        );
        assert_eq!(reply(&[0x03, 0x02, 0x01, 0x10]), None);
        assert_eq!(reply(&[0x02, 0x07, 0x00]), None);
        assert!(echoed(Control::Sidetone, 40, &[0x62, 0x05]));
        assert!(!echoed(Control::Sidetone, 40, &[0x62, 0x04]));
        assert!(echoed(Control::MicVolume, 9, &[0x62, 0x04, 0x00]));
        assert!(echoed(Control::NoiseGate, 1, &[0x01]));
        assert!(!echoed(Control::NoiseGate, 1, &[0x03]));
        assert!(echoed(Control::EqPreset, 3, &[0x67, 0x03]));
        assert!(!echoed(Control::EqPreset, 3, &[0x67, 0x01]));
    }

    #[test]
    fn gen4_reads() {
        assert_eq!(gen4::slider(&[0x68, 0x05, 55, 50], 0x05), Some(55));
        assert_eq!(gen4::slider(&[0x68, 0x04, 55, 50], 0x05), None);
        assert_eq!(gen4::slider(&[0x68, 0x05, 255, 50], 0x05), None);
        assert_eq!(gen4::gate(&[0x6A, 0x03, 0x00]), Some(3));
        assert_eq!(gen4::gate(&[0x6A, 0x04, 0x00]), None);
        assert_eq!(gen4::preset(&[0x02]), Some(2));
        assert_eq!(gen4::preset(&[0x00]), None);
        assert_eq!(gen4::battery(&[0xD0]), Some((80, true)));
        assert_eq!(gen4::battery(&[0x32]), Some((50, false)));
        assert_eq!(gen4::battery(&[0x7F]), None);
        assert_eq!(gen4::balance(&[130]), Some((100, 100)));
        assert_eq!(gen4::balance(&[125]), Some((100, 100)));
        assert_eq!(gen4::balance(&[0]), Some((0, 100)));
        assert_eq!(gen4::balance(&[255]), Some((100, 0)));
        assert_eq!(gen4::balance(&[51]), Some((40, 100)));
        assert!(gen4::balance(&[]).is_none());
    }

    #[test]
    fn gen5_frames() {
        assert_eq!(
            gen5::write(Control::Sidetone, 1).unwrap(),
            [0x02, 0x0C, 0x06, 0x00, 0x09, 0x1C, 0x01, 0xFF, 0x06]
        );
        assert_eq!(
            gen5::write(Control::Sidetone, 0).unwrap(),
            [0x02, 0x0C, 0x06, 0x00, 0x09, 0x1C, 0x01, 0xFF, 0x00]
        );
        assert_eq!(
            gen5::write(Control::NoiseGate, 4).unwrap(),
            [0x02, 0x0C, 0x04, 0x00, 0x14, 0x2D, 0x04]
        );
        assert_eq!(
            gen5::frame(0x06, 0x0C, &[]),
            [0x02, 0x0C, 0x03, 0x00, 0x06, 0x0C]
        );
        let mut r = [0u8; 64];
        r[..9].copy_from_slice(&[0x02, 0x0C, 0x06, 0x00, 0x06, 0x00, 0x50, 0x50, 0x01]);
        assert_eq!(gen5::battery(&r), Some((80, true)));
        r[..7].copy_from_slice(&[0x02, 0x0C, 0x04, 0x00, 0x0A, 0x0C, 0x06]);
        assert_eq!(gen5::mix(&r), Some((100, 100)));
        r[6] = 0;
        assert_eq!(gen5::mix(&r), Some((0, 100)));
        r[6] = 12;
        assert_eq!(gen5::mix(&r), Some((100, 0)));
        r[6] = 3;
        assert_eq!(gen5::mix(&r), Some((50, 100)));
        assert_eq!(gen5::battery(&r), None);
        r[2] = 0x03;
        assert_eq!(gen5::mix(&r), None);
    }
}
