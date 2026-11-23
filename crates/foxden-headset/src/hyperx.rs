//! HyperX wireless headsets, HP and Kingston era ids. Each protocol has its own frames.

use std::time::Duration;

use foxden_battery::hid::{Coll, Dev};

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

const HP: u16 = 0x03F0;
const KINGSTON: u16 = 0x0951;
const WAIT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    /// `21 BB <cmd> <value>`.
    Alpha,
    /// A 12-byte audio block, read and written whole.
    Alpha2,
    /// `06 FF BB <cmd> <value>`.
    CloudII,
    /// `06 00 02 00 9A 00 00 68 4A 8E 0A 00 00 00 BB <cmd> <value>`.
    Kingston,
    /// `66 <cmd> <value>`; reads have bit 7 set.
    Set66,
}

const ROWS: &[(u16, u16, &str, Proto)] = &[
    (HP, 0x098D, "Cloud Alpha Wireless", Proto::Alpha),
    (HP, 0x08BE, "Cloud Alpha 2 Wireless", Proto::Alpha2),
    (HP, 0x0696, "Cloud II Wireless", Proto::CloudII),
    (HP, 0x018B, "Cloud II Wireless", Proto::CloudII),
    (HP, 0x0D93, "Cloud II Wireless", Proto::CloudII),
    (KINGSTON, 0x1718, "Cloud II Wireless", Proto::Kingston),
    (KINGSTON, 0x16EA, "Cloud Flight S", Proto::Kingston),
    (KINGSTON, 0x16EB, "Cloud Flight S", Proto::Kingston),
    (HP, 0x05B7, "Cloud III Wireless", Proto::Set66),
    (HP, 0x0C9D, "Cloud III Wireless", Proto::Set66),
    (HP, 0x069F, "Cloud II Core Wireless", Proto::Set66),
    (HP, 0x0995, "Cloud II Core Wireless", Proto::Set66),
];

fn row(vid: u16, pid: u16) -> Option<(&'static str, Proto)> {
    ROWS.iter()
        .find(|r| r.0 == vid && r.1 == pid)
        .map(|r| (r.2, r.3))
}

pub fn proto(m: &Model) -> Option<Proto> {
    row(m.vid, m.pid).map(|r| r.1)
}

pub fn model(vid: u16, pid: u16) -> Option<Model> {
    let (name, _) = row(vid, pid)?;
    Some(Model {
        vid,
        pid,
        name,
        family: Family::HyperX,
        sidetone: false,
        sidetone_max: 0,
        mic: false,
        anc: false,
        wireless: true,
    })
}

/// The control collection, as the battery reader picks it.
pub fn fits(m: &Model, c: &Coll) -> bool {
    let vendor = c.page >= 0xFF00;
    match (proto(m), m.pid) {
        (Some(Proto::Alpha), _) => vendor && c.out_len >= 3,
        (Some(Proto::Alpha2), _) => c.page == 0xFF13 && c.usage == 0xFF00,
        (Some(Proto::CloudII), _) => c.page == 0xFF90 && c.usage == 0x0303,
        (Some(Proto::Kingston), 0x1718) => c.out_len == 62,
        (Some(Proto::Kingston), _) => {
            c.page == 0xFF13 && c.usage == 0x0001 && c.in_len == 62 && c.out_len == 62
        }
        (Some(Proto::Set66), 0x05B7 | 0x0C9D) => c.out_len == 62 && c.in_len == 62,
        (Some(Proto::Set66), _) => vendor && c.out_len >= 62 && c.in_len >= 62,
        (None, _) => false,
    }
}

const OFF_PRESETS: [i32; 5] = [0, 10, 20, 30, 60];
const OFF_PRESETS_30: [i32; 4] = [0, 10, 20, 30];
pub const ALPHA2_PRESETS: [(i32, &str); 3] = [(0, "Preset 1"), (1, "Preset 2"), (2, "Preset 3")];

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let off = |max: i32, presets: &[i32]| Shape::Minutes {
        max,
        presets: presets.to_vec(),
    };
    match proto(m) {
        Some(Proto::Alpha) => vec![
            (Control::Sidetone, Shape::Steps { min: 0, max: 128 }),
            (Control::AutoOff, off(255, &OFF_PRESETS)),
            (Control::VoicePrompts, Shape::Switch),
            (Control::MicMute, Shape::Switch),
        ],
        Some(Proto::Alpha2) => vec![
            (Control::Sidetone, Shape::Steps { min: 0, max: 20 }),
            (
                Control::EqPreset,
                Shape::Choice {
                    options: ALPHA2_PRESETS.to_vec(),
                },
            ),
        ],
        Some(Proto::CloudII) => vec![
            (Control::Sidetone, Shape::Switch),
            (Control::AutoOff, off(255, &OFF_PRESETS)),
        ],
        Some(Proto::Kingston) => vec![
            (Control::Sidetone, Shape::Switch),
            (Control::AutoOff, off(30, &OFF_PRESETS_30)),
        ],
        Some(Proto::Set66) => vec![
            (Control::Sidetone, Shape::Switch),
            (Control::AutoOff, off(30, &OFF_PRESETS_30)),
            (Control::MicMute, Shape::Switch),
        ],
        None => Vec::new(),
    }
}

const KINGSTON_HEAD: [u8; 15] = [
    0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00, 0xBB,
];

fn kingston(cmd: u8, v: u8) -> Vec<u8> {
    let mut f = KINGSTON_HEAD.to_vec();
    f.extend([cmd, v]);
    f
}

/// A request without a value.
fn ask(p: Proto, cmd: u8) -> Vec<u8> {
    match p {
        Proto::Alpha => vec![0x21, 0xBB, cmd],
        Proto::CloudII => vec![0x06, 0xFF, 0xBB, cmd],
        Proto::Kingston => kingston(cmd, 0),
        Proto::Set66 => vec![0x66, cmd],
        Proto::Alpha2 => vec![cmd, 0x02],
    }
}

/// Whether `r` answers `cmd`, and where its value sits.
fn answer(p: Proto, cmd: u8, r: &[u8]) -> Option<usize> {
    let ok = match p {
        Proto::Alpha => r.len() >= 4 && r[..3] == [0x21, 0xBB, cmd],
        Proto::CloudII => r.len() >= 8 && r[..4] == [0x06, 0xFF, 0xBB, cmd],
        Proto::Kingston => r.len() >= 8 && r[0] == 0x0B && r[2] == 0xBB && r[3] == cmd,
        Proto::Set66 => r.len() >= 5 && r[0] == 0x66 && r[1] == cmd,
        Proto::Alpha2 => r.len() >= 16 && r[0] == cmd + 1 && r[1] == 0x02,
    };
    ok.then_some(match p {
        Proto::Alpha => 3,
        Proto::CloudII | Proto::Kingston => 4,
        Proto::Set66 | Proto::Alpha2 => 2,
    })
}

/// The frames that set `c` to `v` on every protocol but the Cloud Alpha 2.
pub fn frames(m: &Model, c: Control, v: i32) -> Result<Vec<Vec<u8>>, String> {
    let shape = controls(m)
        .into_iter()
        .find(|(k, _)| *k == c)
        .map(|(_, s)| s)
        .ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    if !shape.accepts(v) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    let b = v as u8;
    let p = proto(m).ok_or("not a HyperX headset")?;
    if p == Proto::Alpha2 {
        return Err(format!(
            "{} takes {} through its audio block",
            m.name,
            c.id()
        ));
    }
    let set = |cmd: u8| match p {
        Proto::Alpha => vec![0x21, 0xBB, cmd, b],
        Proto::CloudII => vec![0x06, 0xFF, 0xBB, cmd, b],
        Proto::Kingston => kingston(cmd, b),
        _ => vec![0x66, cmd, b],
    };
    Ok(match (p, c) {
        (Proto::Alpha, Control::Sidetone) if v == 0 => vec![set(0x10)],
        (Proto::Alpha, Control::Sidetone) => vec![vec![0x21, 0xBB, 0x10, 0x01], set(0x11)],
        (Proto::Alpha, Control::AutoOff) => vec![set(0x12)],
        (Proto::Alpha, Control::VoicePrompts) => vec![set(0x13)],
        (Proto::Alpha, Control::MicMute) => vec![set(0x15)],
        (Proto::CloudII, Control::Sidetone) => vec![set(0x21)],
        (Proto::CloudII, Control::AutoOff) => vec![set(0x22)],
        (Proto::Kingston, Control::Sidetone) => vec![set(0x19)],
        (Proto::Kingston, Control::AutoOff) => vec![set(0x18)],
        (Proto::Set66, Control::Sidetone) => vec![set(0x01)],
        (Proto::Set66, Control::AutoOff) => vec![set(0x02)],
        (Proto::Set66, Control::MicMute) => vec![set(0x03)],
        _ => return Err(format!("{} has no {} setting", m.name, c.id())),
    })
}

/// Settings each protocol reads back, by request command.
fn setting_reads(p: Proto) -> &'static [(u8, Control)] {
    match p {
        Proto::Alpha => &[
            (0x07, Control::AutoOff),
            (0x09, Control::VoicePrompts),
            (0x0A, Control::MicMute),
        ],
        Proto::CloudII => &[(0x06, Control::Sidetone), (0x07, Control::AutoOff)],
        Proto::Kingston => &[(0x1A, Control::AutoOff)],
        Proto::Set66 => &[
            (0x84, Control::Sidetone),
            (0x85, Control::AutoOff),
            (0x86, Control::MicMute),
        ],
        Proto::Alpha2 => &[],
    }
}

/// A value read back for `c`, if the model takes it and it is in range.
fn checked(m: &Model, c: Control, v: u8) -> Option<(Control, i32)> {
    let v = v as i32;
    controls(m)
        .iter()
        .any(|(k, s)| *k == c && s.accepts(v))
        .then_some((c, v))
}

/// The Cloud Alpha's sidetone from its on/off and level replies.
pub fn alpha_sidetone(on: Option<u8>, level: Option<u8>) -> Option<(Control, i32)> {
    match (on?, level) {
        (0, _) => Some((Control::Sidetone, 0)),
        (1, Some(l)) if l <= 128 => Some((Control::Sidetone, l as i32)),
        _ => None,
    }
}

/// Status from the battery-type replies: link state, charging and level.
pub fn status_of(
    p: Proto,
    link: Option<&[u8]>,
    charge: Option<&[u8]>,
    level: Option<&[u8]>,
) -> Option<Status> {
    let byte = |r: Option<&[u8]>, i: usize| r.and_then(|r| r.get(i).copied());
    let (connected, charging, battery) = match p {
        Proto::Alpha => (
            byte(link, 3)? != 0x01,
            byte(charge, 3) == Some(1),
            byte(level, 3),
        ),
        Proto::Set66 => (
            byte(link, 2)? != 0,
            byte(charge, 2) == Some(1),
            byte(level, 4),
        ),
        Proto::CloudII | Proto::Kingston => {
            let b = byte(level, 7)?;
            (b <= 100, byte(charge, 4) == Some(1), Some(b))
        }
        Proto::Alpha2 => (
            byte(link, 2)? != 0,
            byte(level, 3) == Some(1),
            byte(level, 2),
        ),
    };
    if !connected {
        return Some(Status::default());
    }
    Some(Status {
        connected,
        battery: battery.filter(|b| *b <= 100),
        charging,
        chatmix: None,
        values: Vec::new(),
    })
}

pub const BLOCK: usize = 12;
const ST_ON: usize = 4;
const ST_LEVEL: usize = 5;
const BALANCE: usize = 8;
const PRESET: usize = 11;

/// The audio block in a `61 02` reply, if every byte FoxDen changes holds a known value.
pub fn alpha2_block(r: &[u8]) -> Option<[u8; BLOCK]> {
    if r.len() < 4 + BLOCK || r[..2] != [0x61, 0x02] {
        return None;
    }
    let b: [u8; BLOCK] = r[4..4 + BLOCK].try_into().ok()?;
    (b[ST_ON] <= 1 && b[ST_LEVEL] <= 20 && (1..=3).contains(&b[PRESET])).then_some(b)
}

pub fn alpha2_values(b: &[u8; BLOCK]) -> Vec<(Control, i32)> {
    let sidetone = if b[ST_ON] == 0 { 0 } else { b[ST_LEVEL] as i32 };
    vec![
        (Control::Sidetone, sidetone),
        (Control::EqPreset, b[PRESET] as i32 - 1),
    ]
}

/// Game and chat, 0..=100, from the balance byte: 0x7B all game, 0x80 even, 0x85 all chat.
pub fn alpha2_chatmix(b: &[u8; BLOCK]) -> Option<(u8, u8)> {
    let step = b[BALANCE] as i32 - 0x80;
    (-5..=5).contains(&step).then(|| {
        let game = if step > 0 { 100 - step * 20 } else { 100 };
        let chat = if step < 0 { 100 + step * 20 } else { 100 };
        (game as u8, chat as u8)
    })
}

/// The `60 01` frame that writes `b` back with `c` set to `v`.
pub fn alpha2_frame(m: &Model, b: &[u8; BLOCK], c: Control, v: i32) -> Result<Vec<u8>, String> {
    if !controls(m).iter().any(|(k, s)| *k == c && s.accepts(v)) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    let mut b = *b;
    match c {
        Control::Sidetone if v == 0 => b[ST_ON] = 0,
        Control::Sidetone => {
            b[ST_ON] = 1;
            b[ST_LEVEL] = v as u8;
        }
        Control::EqPreset => b[PRESET] = v as u8 + 1,
        _ => return Err(format!("{} has no {} setting", m.name, c.id())),
    }
    let mut f = vec![0x60, 0x01];
    f.extend(b);
    Ok(f)
}

/// The base station's answer to a `60 01` write: `Some(true)` taken, `Some(false)` refused.
pub fn alpha2_ack(r: &[u8]) -> Option<bool> {
    (r.len() >= 16 && r[0] == 0xFF && r[14] == 0x60 && r[15] == 0x01).then_some(r[1] == 0x01)
}

fn query(dev: &Dev, p: Proto, req: &[u8], cmd: u8) -> Option<Vec<u8>> {
    if p == Proto::Kingston {
        let _ = dev.get_input_report(0x06);
    }
    dev.flush();
    if !dev.write(req) {
        return None;
    }
    dev.read_until(WAIT, |r| answer(p, cmd, r).map(|_| r.to_vec()))
}

fn value(dev: &Dev, p: Proto, cmd: u8) -> Option<u8> {
    let r = query(dev, p, &ask(p, cmd), cmd)?;
    answer(p, cmd, &r).map(|i| r[i])
}

fn read_status(dev: &Dev, p: Proto) -> Option<Status> {
    let q = |cmd: u8, sub: u8| -> Option<Vec<u8>> {
        let req = if p == Proto::Alpha2 {
            vec![cmd, sub]
        } else {
            ask(p, cmd)
        };
        let want = |r: &[u8]| match p {
            Proto::Alpha2 => r.len() >= 8 && r[0] == cmd + 1 && r[1] == sub,
            _ => answer(p, cmd, r).is_some(),
        };
        if p == Proto::Kingston {
            let _ = dev.get_input_report(0x06);
        }
        dev.flush();
        if !dev.write(&req) {
            return None;
        }
        dev.read_until(WAIT, |r| want(r).then(|| r.to_vec()))
    };
    match p {
        Proto::Alpha => {
            let link = q(0x03, 0)?;
            let on = status_of(p, Some(&link), None, None)?.connected;
            let (c, l) = if on {
                (q(0x0C, 0), q(0x0B, 0))
            } else {
                (None, None)
            };
            status_of(p, Some(&link), c.as_deref(), l.as_deref())
        }
        Proto::Set66 => {
            let link = q(0x82, 0)?;
            let on = status_of(p, Some(&link), None, None)?.connected;
            let (c, l) = if on {
                (q(0x8A, 0), q(0x89, 0))
            } else {
                (None, None)
            };
            status_of(p, Some(&link), c.as_deref(), l.as_deref())
        }
        Proto::CloudII | Proto::Kingston => {
            let level = q(0x02, 0)?;
            status_of(p, None, q(0x03, 0).as_deref(), Some(&level))
        }
        Proto::Alpha2 => {
            let link = q(0x52, 0x01)?;
            let on = status_of(p, Some(&link), None, None)?.connected;
            let level = if on { q(0x50, 0x02) } else { None };
            status_of(p, Some(&link), None, level.as_deref())
        }
    }
}

fn audio_block(dev: &Dev) -> Option<[u8; BLOCK]> {
    alpha2_block(&query(dev, Proto::Alpha2, &[0x60, 0x02], 0x60)?)
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    let p = proto(&f.model)?;
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let mut status = read_status(&dev, p);
    let mut values = Vec::new();
    if let Some(s) = status.as_mut().filter(|s| s.connected) {
        if p == Proto::Alpha2 {
            if let Some(b) = audio_block(&dev) {
                s.chatmix = alpha2_chatmix(&b);
                if settings {
                    values = alpha2_values(&b);
                }
            }
        } else if settings {
            for &(cmd, c) in setting_reads(p) {
                values.extend(value(&dev, p, cmd).and_then(|v| checked(&f.model, c, v)));
            }
            if p == Proto::Alpha {
                values.extend(alpha_sidetone(value(&dev, p, 0x05), value(&dev, p, 0x06)));
            }
        }
    }
    Some(Reading { status, values })
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    let p = proto(&f.model).ok_or("not a HyperX headset")?;
    let frames = if p == Proto::Alpha2 {
        Vec::new()
    } else {
        frames(&f.model, c, v)?
    };
    let dev = Dev::open(&f.coll).ok_or("the headset's receiver could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    if p == Proto::Alpha2 {
        let b = audio_block(&dev).ok_or("the base station did not report its audio settings")?;
        let frame = alpha2_frame(&f.model, &b, c, v)?;
        dev.flush();
        if !dev.write(&frame) {
            return Err("the base station did not take the setting".into());
        }
        return match dev.read_until(WAIT, alpha2_ack) {
            Some(true) => Ok(()),
            Some(false) => Err("the base station refused the setting".into()),
            None => Err("the base station did not answer".into()),
        };
    }
    for frame in frames {
        if p == Proto::Kingston {
            let _ = dev.get_input_report(0x06);
        }
        if !dev.write(&frame) {
            return Err("the headset's receiver did not take the setting".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hx(vid: u16, pid: u16) -> Model {
        model(vid, pid).unwrap()
    }

    #[test]
    fn table() {
        assert_eq!(hx(HP, 0x098D).name, "Cloud Alpha Wireless");
        assert_eq!(proto(&hx(KINGSTON, 0x16EB)), Some(Proto::Kingston));
        assert!(model(HP, 0x0E90).is_none());
        assert!(model(KINGSTON, 0x098D).is_none());
        let c = Coll {
            page: 0xFF90,
            usage: 0x0303,
            ..Coll::default()
        };
        assert!(fits(&hx(HP, 0x0696), &c));
        assert!(!fits(&hx(HP, 0x08BE), &c));
    }

    #[test]
    fn alpha_frames() {
        let m = hx(HP, 0x098D);
        assert_eq!(
            frames(&m, Control::Sidetone, 0).unwrap(),
            vec![vec![0x21, 0xBB, 0x10, 0x00]]
        );
        assert_eq!(
            frames(&m, Control::Sidetone, 64).unwrap(),
            vec![vec![0x21, 0xBB, 0x10, 0x01], vec![0x21, 0xBB, 0x11, 64]]
        );
        assert_eq!(
            frames(&m, Control::AutoOff, 20).unwrap(),
            vec![vec![0x21, 0xBB, 0x12, 20]]
        );
        assert_eq!(
            frames(&m, Control::VoicePrompts, 1).unwrap(),
            vec![vec![0x21, 0xBB, 0x13, 1]]
        );
        assert_eq!(
            frames(&m, Control::MicMute, 0).unwrap(),
            vec![vec![0x21, 0xBB, 0x15, 0]]
        );
        assert!(frames(&m, Control::Sidetone, 129).is_err());
        assert!(frames(&m, Control::AutoOff, 256).is_err());
        assert!(frames(&m, Control::MicVolume, 1).is_err());
    }

    #[test]
    fn cloud_ii_frames() {
        for pid in [0x0696, 0x018B, 0x0D93] {
            let m = hx(HP, pid);
            assert_eq!(
                frames(&m, Control::Sidetone, 1).unwrap(),
                vec![vec![0x06, 0xFF, 0xBB, 0x21, 1]]
            );
            assert_eq!(
                frames(&m, Control::AutoOff, 60).unwrap(),
                vec![vec![0x06, 0xFF, 0xBB, 0x22, 60]]
            );
            assert!(frames(&m, Control::MicMute, 1).is_err());
        }
    }

    #[test]
    fn kingston_frames() {
        let m = hx(KINGSTON, 0x1718);
        let head = KINGSTON_HEAD.to_vec();
        let with = |tail: [u8; 2]| [head.clone(), tail.to_vec()].concat();
        assert_eq!(
            frames(&m, Control::Sidetone, 1).unwrap(),
            vec![with([0x19, 1])]
        );
        assert_eq!(
            frames(&m, Control::AutoOff, 30).unwrap(),
            vec![with([0x18, 30])]
        );
        assert!(frames(&m, Control::AutoOff, 31).is_err());
        assert_eq!(with([0x18, 30])[..5], [0x06, 0x00, 0x02, 0x00, 0x9A]);
        assert_eq!(with([0x18, 30]).len(), 17);
        assert_eq!(ask(Proto::Kingston, 0x1A), with([0x1A, 0]));
    }

    #[test]
    fn set66_frames() {
        for pid in [0x05B7, 0x0C9D, 0x069F, 0x0995] {
            let m = hx(HP, pid);
            assert_eq!(
                frames(&m, Control::Sidetone, 1).unwrap(),
                vec![vec![0x66, 0x01, 1]]
            );
            assert_eq!(
                frames(&m, Control::AutoOff, 10).unwrap(),
                vec![vec![0x66, 0x02, 10]]
            );
            assert_eq!(
                frames(&m, Control::MicMute, 1).unwrap(),
                vec![vec![0x66, 0x03, 1]]
            );
            assert!(frames(&m, Control::VoicePrompts, 1).is_err());
        }
    }

    #[test]
    fn replies() {
        let a = hx(HP, 0x098D);
        assert_eq!(answer(Proto::Alpha, 0x07, &[0x21, 0xBB, 0x07, 15]), Some(3));
        assert_eq!(answer(Proto::Alpha, 0x07, &[0x21, 0xBB, 0x24, 15]), None);
        assert_eq!(
            checked(&a, Control::AutoOff, 15),
            Some((Control::AutoOff, 15))
        );
        assert_eq!(checked(&a, Control::MicMute, 2), None);
        assert_eq!(
            alpha_sidetone(Some(0), Some(90)),
            Some((Control::Sidetone, 0))
        );
        assert_eq!(
            alpha_sidetone(Some(1), Some(90)),
            Some((Control::Sidetone, 90))
        );
        assert_eq!(alpha_sidetone(Some(1), Some(200)), None);
        assert_eq!(alpha_sidetone(None, Some(5)), None);
        assert_eq!(
            answer(Proto::CloudII, 0x07, &[0x06, 0xFF, 0xBB, 0x07, 10, 0, 0, 0]),
            Some(4)
        );
        let k = [0x0B, 0x00, 0xBB, 0x1A, 20, 0, 0, 0];
        assert_eq!(answer(Proto::Kingston, 0x1A, &k), Some(4));
        assert_eq!(answer(Proto::Kingston, 0x02, &k), None);
        assert_eq!(answer(Proto::Set66, 0x84, &[0x66, 0x84, 1, 0, 0]), Some(2));
        assert_eq!(answer(Proto::Set66, 0x84, &[0x66, 0x09, 1, 0, 0]), None);
        let k66 = hx(HP, 0x05B7);
        assert_eq!(checked(&k66, Control::AutoOff, 45), None);
    }

    #[test]
    fn status_replies() {
        let s = status_of(
            Proto::Alpha,
            Some(&[0x21, 0xBB, 0x03, 0x02]),
            Some(&[0x21, 0xBB, 0x0C, 0x01]),
            Some(&[0x21, 0xBB, 0x0B, 70]),
        )
        .unwrap();
        assert_eq!((s.connected, s.charging, s.battery), (true, true, Some(70)));
        let off = status_of(Proto::Alpha, Some(&[0x21, 0xBB, 0x03, 0x01]), None, None).unwrap();
        assert!(!off.connected);
        let s = status_of(
            Proto::Set66,
            Some(&[0x66, 0x82, 1, 0, 0]),
            Some(&[0x66, 0x8A, 0, 0, 0]),
            Some(&[0x66, 0x89, 0x0F, 0xA0, 55]),
        )
        .unwrap();
        assert_eq!(
            (s.connected, s.charging, s.battery),
            (true, false, Some(55))
        );
        let lvl = [0x06, 0xFF, 0xBB, 0x02, 0, 0x0F, 0xA0, 80];
        let s = status_of(Proto::CloudII, None, None, Some(&lvl)).unwrap();
        assert_eq!((s.connected, s.battery), (true, Some(80)));
        let s = status_of(
            Proto::Alpha2,
            Some(&[0x53, 0x01, 1]),
            None,
            Some(&[0x51, 0x02, 40, 1, 0, 0, 0x10, 0x0F]),
        )
        .unwrap();
        assert_eq!((s.charging, s.battery), (true, Some(40)));
        assert_eq!(status_of(Proto::Alpha2, None, None, None), None);
    }

    fn alpha2_reply(block: [u8; BLOCK]) -> Vec<u8> {
        let mut r = vec![0x61, 0x02, 0xAA, 0xBB];
        r.extend(block);
        r.resize(64, 0);
        r
    }

    #[test]
    fn alpha2_block_round_trip() {
        let m = hx(HP, 0x08BE);
        let block = [9, 8, 7, 6, 1, 12, 5, 4, 0x82, 3, 2, 2];
        let b = alpha2_block(&alpha2_reply(block)).unwrap();
        assert_eq!(b, block);
        assert_eq!(
            alpha2_values(&b),
            vec![(Control::Sidetone, 12), (Control::EqPreset, 1)]
        );
        assert_eq!(alpha2_chatmix(&b), Some((60, 100)));
        let f = alpha2_frame(&m, &b, Control::Sidetone, 0).unwrap();
        assert_eq!(f, vec![0x60, 0x01, 9, 8, 7, 6, 0, 12, 5, 4, 0x82, 3, 2, 2]);
        let f = alpha2_frame(&m, &b, Control::Sidetone, 20).unwrap();
        assert_eq!(f, vec![0x60, 0x01, 9, 8, 7, 6, 1, 20, 5, 4, 0x82, 3, 2, 2]);
        let f = alpha2_frame(&m, &b, Control::EqPreset, 2).unwrap();
        assert_eq!(f, vec![0x60, 0x01, 9, 8, 7, 6, 1, 12, 5, 4, 0x82, 3, 2, 3]);
        assert!(alpha2_frame(&m, &b, Control::Sidetone, 21).is_err());
        assert!(alpha2_frame(&m, &b, Control::EqPreset, 3).is_err());
        assert!(frames(&m, Control::Sidetone, 1).is_err());
    }

    #[test]
    fn alpha2_checks() {
        let mut block = [0, 0, 0, 0, 0, 5, 0, 0, 0x80, 0, 0, 1];
        assert_eq!(alpha2_chatmix(&block), Some((100, 100)));
        block[BALANCE] = 0x7B;
        assert_eq!(alpha2_chatmix(&block), Some((100, 0)));
        block[BALANCE] = 0x90;
        assert_eq!(alpha2_chatmix(&block), None);
        assert_eq!(alpha2_values(&block)[0], (Control::Sidetone, 0));
        for (i, bad) in [(ST_ON, 2), (ST_LEVEL, 21), (PRESET, 0), (PRESET, 4)] {
            let mut b = block;
            b[i] = bad;
            assert_eq!(alpha2_block(&alpha2_reply(b)), None);
        }
        assert_eq!(alpha2_block(&[0x51, 0x02, 0, 0]), None);
        let mut ack = [0u8; 16];
        ack[0] = 0xFF;
        ack[1] = 0x01;
        ack[14] = 0x60;
        ack[15] = 0x01;
        assert_eq!(alpha2_ack(&ack), Some(true));
        ack[1] = 0x02;
        assert_eq!(alpha2_ack(&ack), Some(false));
        ack[14] = 0x50;
        assert_eq!(alpha2_ack(&ack), None);
    }
}
