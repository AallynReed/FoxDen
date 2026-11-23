//! Settings of Razer headsets. A setting's register is its read register with bit 7 set; each
//! family wraps the registers in its own frame.

use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::sleep;
use std::time::Duration;

use foxden_battery::hid::{Coll, Dev};
use foxden_battery::util::xor8;

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x1532;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wire {
    /// Report 2: `02 00 seq 00 00 00 4+n 00 00 dir reg 00 n args..`, xor of 0..62 at 62.
    HyperSpeed,
    /// The HyperSpeed frame with a fixed sequence byte.
    V3Pro,
    /// Report 2: `02 80 8+n 00 00 'P' 'A' 08 00 3+n reg 00 n args..`, behind a remote switch.
    Pa,
    /// Feature report 7: `07 00 txid 00 00 00 n class cmd args..`, xor of 3..=60 at 62.
    V3x,
}

const MODELS: &[(u16, &str, Wire)] = &[
    (0x0565, "BlackShark V2 HyperSpeed", Wire::HyperSpeed),
    (0x0577, "BlackShark V3 Pro", Wire::V3Pro),
    (0x0A55, "BlackShark V3 Pro for Xbox", Wire::V3Pro),
    (0x0555, "BlackShark V2 Pro (2023)", Wire::Pa),
    (0x057D, "BlackShark V3 X", Wire::V3x),
    (0x057C, "BlackShark V3 X", Wire::V3x),
    (0x0A5B, "BlackShark V3 X for Xbox", Wire::V3x),
];

fn wire(pid: u16) -> Option<Wire> {
    MODELS.iter().find(|m| m.0 == pid).map(|m| m.2)
}

pub fn model(pid: u16) -> Option<Model> {
    let &(_, name, w) = MODELS.iter().find(|m| m.0 == pid)?;
    Some(Model {
        vid: VID,
        pid,
        name,
        family: Family::Razer,
        sidetone: true,
        sidetone_max: if w == Wire::Pa { 10 } else { 15 },
        mic: false,
        anc: w == Wire::V3Pro,
        wireless: pid != 0x057C,
    })
}

const HS_EQ: [(i32, &str); 4] = [(0, "Flat"), (7, "Game"), (8, "Music"), (9, "Movie")];
const V3X_EQ: [(i32, &str); 4] = [(0, "Default"), (1, "Game"), (2, "Movie"), (3, "Music")];
const QUARTERS: [(i32, &str); 5] = [
    (0, "Never"),
    (15, "15 min"),
    (30, "30 min"),
    (45, "45 min"),
    (60, "60 min"),
];
const V3X_OFF: [(i32, &str); 3] = [(30, "30 min"), (45, "45 min"), (60, "60 min")];

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let sidetone = (
        Control::Sidetone,
        Shape::Steps {
            min: 0,
            max: m.sidetone_max as i32,
        },
    );
    let choice = |c, options: &[(i32, &'static str)]| {
        (
            c,
            Shape::Choice {
                options: options.to_vec(),
            },
        )
    };
    match wire(m.pid) {
        Some(Wire::HyperSpeed) => vec![
            sidetone,
            (
                Control::AutoOff,
                Shape::Minutes {
                    max: 255,
                    presets: vec![0, 5, 10, 15, 30, 60, 90],
                },
            ),
            choice(Control::EqPreset, &HS_EQ),
        ],
        Some(Wire::V3Pro) => vec![
            sidetone,
            (Control::Anc, Shape::Steps { min: 0, max: 4 }),
            choice(Control::AutoOff, &QUARTERS),
        ],
        Some(Wire::Pa) => vec![sidetone, choice(Control::AutoOff, &QUARTERS)],
        Some(Wire::V3x) => vec![
            sidetone,
            choice(Control::EqPreset, &V3X_EQ),
            choice(Control::AutoOff, &V3X_OFF),
        ],
        None => Vec::new(),
    }
}

/// One write: `class` is only used by the V3 X.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub class: u8,
    pub reg: u8,
    pub args: Vec<u8>,
}

fn cmd(reg: u8, args: &[u8]) -> Cmd {
    Cmd {
        class: 0,
        reg,
        args: args.to_vec(),
    }
}

const SIDETONE_ON: u8 = 0x98;
const SIDETONE_LEVEL: u8 = 0x99;
const ANC: u8 = 0x92;
const EQ_PRESET: u8 = 0x93;
const EQ_ON: u8 = 0x9E;
const AUTO_OFF: u8 = 0xAC;

/// The writes that set `c` to `v`, in order. `Err` for a control the model lacks or a value out of range.
pub fn commands(m: &Model, c: Control, v: i32) -> Result<Vec<Cmd>, String> {
    let shape = controls(m)
        .into_iter()
        .find(|(k, _)| *k == c)
        .map(|(_, s)| s)
        .ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    if !shape.accepts(v) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    let b = v as u8;
    let w = wire(m.pid).ok_or("not a Razer headset")?;
    Ok(match (w, c) {
        (Wire::V3x, Control::Sidetone) => vec![Cmd {
            class: 0x18,
            reg: 0x03,
            args: vec![0, 3, b],
        }],
        (Wire::V3x, Control::EqPreset) => vec![Cmd {
            class: 0x18,
            reg: 0x00,
            args: vec![0, 1, 0, b],
        }],
        (Wire::V3x, Control::AutoOff) => vec![Cmd {
            class: 0x07,
            reg: 0x03,
            args: ((v * 60) as u16).to_be_bytes().to_vec(),
        }],
        (_, Control::Sidetone) if v == 0 => vec![cmd(SIDETONE_ON, &[0])],
        (Wire::HyperSpeed, Control::Sidetone) => {
            vec![cmd(SIDETONE_LEVEL, &[b]), cmd(SIDETONE_ON, &[1])]
        }
        (_, Control::Sidetone) => vec![cmd(SIDETONE_ON, &[1]), cmd(SIDETONE_LEVEL, &[b])],
        (_, Control::Anc) if v == 0 => vec![cmd(ANC, &[0, 1])],
        (_, Control::Anc) => vec![cmd(ANC, &[1, b])],
        (_, Control::EqPreset) => vec![cmd(EQ_PRESET, &[b]), cmd(EQ_ON, &[1])],
        (_, Control::AutoOff) => vec![cmd(AUTO_OFF, &[b])],
        _ => return Err(format!("{} has no {} setting", m.name, c.id())),
    })
}

/// `dir` 0x80 reaches the headset, 0x00 the receiver.
pub fn hs_frame(seq: u8, dir: u8, reg: u8, args: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x02;
    b[2] = seq;
    b[6] = 4 + args.len() as u8;
    b[9] = dir;
    b[10] = reg;
    b[12] = args.len() as u8;
    b[13..13 + args.len()].copy_from_slice(args);
    b[62] = xor8(&b[..62]);
    b
}

/// Status byte and payload of the reply to `reg`; with `seq`, only the reply to that request.
pub fn hs_reply(reg: u8, seq: Option<u8>, r: &[u8]) -> Option<(u8, Vec<u8>)> {
    if r.len() < 14 || r[0] != 0x02 || r[10] != reg || r[11] != 0x01 {
        return None;
    }
    if seq.is_some_and(|s| r.len() < 64 || (r[2] ^ s) & 0x1F != 0 || xor8(&r[1..62]) != r[62]) {
        return None;
    }
    let end = (13 + r[12] as usize).min(r.len());
    Some((r[1], r[13..end].to_vec()))
}

pub fn pa_frame(reg: u8, args: &[u8]) -> [u8; 64] {
    let n = args.len() as u8;
    let mut b = [0u8; 64];
    b[..13].copy_from_slice(&[
        0x02,
        0x80,
        8 + n,
        0,
        0,
        b'P',
        b'A',
        0x08,
        0,
        3 + n,
        reg,
        0,
        n,
    ]);
    b[13..13 + args.len()].copy_from_slice(args);
    b
}

/// Routes the receiver's commands to the headset, or back.
pub fn pa_remote(on: bool) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..12].copy_from_slice(&[
        0x02, 0x80, 0x07, 0, 0, b'P', b'A', 0x0E, 0, 0x02, 0xE1, on as u8,
    ]);
    b
}

pub fn pa_reply(reg: u8, r: &[u8]) -> Option<u8> {
    (r.len() >= 16 && r[0] == 0x02 && r[12] == reg && r[13] == 0x01 && r[14] >= 1).then(|| r[15])
}

pub fn v3x_frame(txid: u8, c: &Cmd) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x07;
    b[2] = txid;
    b[6] = c.args.len() as u8;
    b[7] = c.class;
    b[8] = c.reg;
    b[9..9 + c.args.len()].copy_from_slice(&c.args);
    b[62] = xor8(&b[3..=60]);
    b
}

/// `Some(true)` when the headset took the command, `Some(false)` when it refused it.
pub fn v3x_reply(txid: u8, c: &Cmd, r: &[u8]) -> Option<bool> {
    (r.len() >= 63 && r[2] == txid && r[7] == c.class && r[8] == c.reg)
        .then(|| matches!(r[1], 0x01 | 0x02))
}

pub fn fits(m: &Model, c: &Coll) -> bool {
    match wire(m.pid) {
        Some(Wire::HyperSpeed) => {
            c.page == 0xFF14 && c.usage == 0x01 && c.out_len >= 64 && c.in_len >= 64
        }
        Some(Wire::V3Pro) => c.iface == 5 && c.out_len >= 64 && c.in_len >= 15,
        Some(Wire::Pa) => c.iface == 3 && c.page == 0xFF00 && c.out_len >= 64 && c.in_len >= 16,
        Some(Wire::V3x) => c.iface == 0 && c.feat_len >= 64,
        None => false,
    }
}

static SEQ: AtomicU8 = AtomicU8::new(0);
static TXID: AtomicU8 = AtomicU8::new(0x80);
const HS_WAIT: Duration = Duration::from_millis(650);
const STATUS_OK: u8 = 0x02;
const STATUS_BUSY: u8 = 0x03;

fn hs_ask(dev: &Dev, w: Wire, dir: u8, reg: u8, args: &[u8]) -> Option<(u8, Vec<u8>)> {
    for _ in 0..3 {
        let seq = match w {
            Wire::HyperSpeed => 0x60 | (SEQ.fetch_add(1, Ordering::Relaxed) & 0x1F),
            _ => 0x60,
        };
        let only = (w == Wire::HyperSpeed).then_some(seq);
        dev.flush();
        if !dev.write(&hs_frame(seq, dir, reg, args)) {
            return None;
        }
        let got = dev.read_until(HS_WAIT, |r| hs_reply(reg, only, r))?;
        if got.0 != STATUS_BUSY {
            return Some(got);
        }
        sleep(Duration::from_millis(150));
    }
    None
}

fn hs_get(dev: &Dev, w: Wire, dir: u8, reg: u8) -> Option<u8> {
    hs_ask(dev, w, dir, reg, &[])
        .filter(|(s, _)| *s == STATUS_OK)
        .and_then(|(_, p)| p.first().copied())
}

fn pa_get(dev: &Dev, reg: u8) -> Option<u8> {
    dev.flush();
    for _ in 0..3 {
        dev.write(&pa_remote(true));
        sleep(Duration::from_millis(35));
        if !dev.write(&pa_frame(reg, &[])) {
            return None;
        }
        if let Some(v) = dev.read_until(Duration::from_millis(150), |r| pa_reply(reg, r)) {
            return Some(v);
        }
    }
    None
}

fn sidetone(on: Option<u8>, level: Option<u8>, max: u8) -> Option<(Control, i32)> {
    match (on?, level) {
        (0, _) => Some((Control::Sidetone, 0)),
        (1, Some(l)) if l <= max => Some((Control::Sidetone, l as i32)),
        _ => None,
    }
}

fn off() -> Reading {
    Reading {
        status: Some(Status::default()),
        values: Vec::new(),
    }
}

/// The V3 family loses its link when asked often, so its status is the battery poll's.
fn polled(pid: u16) -> Reading {
    let status = match foxden_battery::razer_headset_reading(pid) {
        Some(b) => Status {
            connected: b.connected,
            battery: b.percent,
            charging: b.charging,
            ..Status::default()
        },
        None => Status {
            connected: true,
            ..Status::default()
        },
    };
    Reading {
        status: Some(status),
        values: Vec::new(),
    }
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    let m = &f.model;
    let w = wire(m.pid)?;
    if matches!(w, Wire::V3Pro | Wire::V3x) {
        return Some(polled(m.pid));
    }
    let dev = Dev::open(&f.coll)?;
    let _turn = foxden_core::hid_turn::take();
    let get = |reg| match w {
        Wire::Pa => pa_get(&dev, reg),
        _ => hs_get(&dev, w, 0x80, reg),
    };
    if w == Wire::HyperSpeed {
        match hs_get(&dev, w, 0x00, 0x20) {
            None => return None,
            Some(0) => return Some(off()),
            Some(_) => {}
        }
    }
    let Some(level) = get(0x21) else {
        if w == Wire::Pa {
            dev.write(&pa_remote(false));
        }
        return Some(off());
    };
    let charging = get(0x2A).is_some_and(|v| v > 0);
    let mut values = Vec::new();
    if settings {
        values.extend(sidetone(get(0x18), get(0x19), m.sidetone_max));
        if let Some(v) = get(0x2C) {
            values.push((Control::AutoOff, v as i32));
        }
        if w == Wire::HyperSpeed
            && let Some(p) = get(0x13).filter(|p| HS_EQ.iter().any(|o| o.0 == *p as i32))
        {
            values.push((Control::EqPreset, p as i32));
        }
    }
    if w == Wire::Pa {
        dev.write(&pa_remote(false));
    }
    Some(Reading {
        status: Some(Status {
            connected: true,
            battery: (level <= 100).then_some(level),
            charging,
            ..Status::default()
        }),
        values,
    })
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    let m = &f.model;
    let w = wire(m.pid).ok_or("not a Razer headset")?;
    let cmds = commands(m, c, v)?;
    let refused = || "the headset did not take the setting".to_string();
    if w == Wire::V3x {
        let dev = Dev::open_feature_only(&f.coll).ok_or("the headset could not be opened")?;
        let _turn = foxden_core::hid_turn::take();
        for c in &cmds {
            if !v3x_send(&dev, c).ok_or("the headset did not answer")? {
                return Err(refused());
            }
        }
        return Ok(());
    }
    let dev = Dev::open(&f.coll).ok_or("the headset's receiver could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    if w == Wire::Pa {
        for c in &cmds {
            dev.write(&pa_remote(true));
            sleep(Duration::from_millis(35));
            let took = dev.write(&pa_frame(c.reg, &c.args));
            sleep(Duration::from_millis(35));
            dev.write(&pa_remote(false));
            if !took {
                return Err(refused());
            }
        }
        return Ok(());
    }
    for c in &cmds {
        let (status, _) =
            hs_ask(&dev, w, 0x80, c.reg, &c.args).ok_or("the headset did not answer")?;
        if status != STATUS_OK {
            return Err(refused());
        }
    }
    Ok(())
}

fn v3x_send(dev: &Dev, c: &Cmd) -> Option<bool> {
    for _ in 0..4 {
        let txid = TXID.fetch_add(1, Ordering::Relaxed);
        if !dev.set_feature(&v3x_frame(txid, c)) {
            return None;
        }
        sleep(Duration::from_millis(60));
        if let Some(took) = dev.get_feature(0x07).and_then(|r| v3x_reply(txid, c, &r)) {
            return Some(took);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }

    fn frames(pid: u16, c: Control, v: i32) -> Vec<[u8; 64]> {
        let m = model(pid).unwrap();
        commands(&m, c, v)
            .unwrap()
            .iter()
            .map(|c| match wire(pid).unwrap() {
                Wire::Pa => pa_frame(c.reg, &c.args),
                Wire::V3x => v3x_frame(0, c),
                _ => hs_frame(0x60, 0x80, c.reg, &c.args),
            })
            .collect()
    }

    #[test]
    fn models() {
        let hs = crate::proto::any_model(VID, 0x0565).unwrap();
        assert_eq!((hs.family, hs.brand()), (Family::Razer, "Razer"));
        assert_eq!(hs.name, "BlackShark V2 HyperSpeed");
        assert!(crate::proto::any_model(VID, 0x0528).is_none());
        assert!(crate::proto::any_model(VID, 0x056E).is_none());
        assert!(model(0x0577).unwrap().anc && !model(0x0565).unwrap().anc);
        assert!(!model(0x057C).unwrap().wireless);
        assert!(!hs.has_eq());
        assert_eq!(hs.save(Some(Control::Sidetone)), crate::proto::Save::None);
        let ids: Vec<_> = model(0x0555)
            .unwrap()
            .controls()
            .into_iter()
            .map(|c| c.0)
            .collect();
        assert_eq!(ids, [Control::Sidetone, Control::AutoOff]);
    }

    #[test]
    fn hyperspeed_frames() {
        let f = frames(0x0565, Control::AutoOff, 30);
        assert_eq!(f.len(), 1);
        assert_eq!(
            f[0][..14],
            [2, 0, 0x60, 0, 0, 0, 5, 0, 0, 0x80, 0xAC, 0, 1, 30]
        );
        assert_eq!(f[0][62], 0x54);
        let f = frames(0x0565, Control::Sidetone, 12);
        assert_eq!((f[0][10], f[0][13]), (0x99, 12));
        assert_eq!((f[1][10], f[1][13]), (0x98, 1));
        let f = frames(0x0565, Control::Sidetone, 0);
        assert_eq!(f.len(), 1);
        assert_eq!((f[0][10], f[0][12], f[0][13]), (0x98, 1, 0));
        let f = frames(0x0565, Control::EqPreset, 8);
        assert_eq!((f[0][10], f[0][13]), (0x93, 8));
        assert_eq!((f[1][10], f[1][13]), (0x9E, 1));
        assert_eq!(hs_frame(0x60, 0x80, 0x21, &[])[62], 0xC7);
    }

    #[test]
    fn hyperspeed_replies() {
        let mut r = hs_frame(0x6A, 0x80, 0x19, &[7]);
        r[1] = 0x02;
        r[11] = 0x01;
        r[62] = xor8(&r[1..62]);
        assert_eq!(hs_reply(0x19, Some(0x6A), &r), Some((2, vec![7])));
        assert_eq!(hs_reply(0x19, Some(0x6B), &r), None);
        assert_eq!(hs_reply(0x18, Some(0x6A), &r), None);
        assert_eq!(hs_reply(0x19, None, &r), Some((2, vec![7])));
        r[13] = 8;
        assert_eq!(hs_reply(0x19, Some(0x6A), &r), None);
        r[11] = 0;
        assert_eq!(hs_reply(0x19, None, &r), None);
        assert_eq!(sidetone(Some(1), Some(7), 15), Some((Control::Sidetone, 7)));
        assert_eq!(sidetone(Some(0), Some(7), 15), Some((Control::Sidetone, 0)));
        assert_eq!(sidetone(Some(1), Some(16), 15), None);
        assert_eq!(sidetone(None, Some(3), 15), None);
    }

    #[test]
    fn v3_pro_frames() {
        let f = frames(0x0577, Control::Sidetone, 15);
        assert_eq!(
            f[0][..14],
            [2, 0, 0x60, 0, 0, 0, 5, 0, 0, 0x80, 0x98, 0, 1, 1]
        );
        assert_eq!(
            f[1][..14],
            [2, 0, 0x60, 0, 0, 0, 5, 0, 0, 0x80, 0x99, 0, 1, 15]
        );
        assert_eq!(f[1][62], xor8(&f[1][..62]));
        let f = frames(0x0A55, Control::Anc, 3);
        assert_eq!(
            f[0][..15],
            [2, 0, 0x60, 0, 0, 0, 6, 0, 0, 0x80, 0x92, 0, 2, 1, 3]
        );
        assert_eq!(frames(0x0577, Control::Anc, 0)[0][13..15], [0, 1]);
        assert_eq!(
            frames(0x0577, Control::AutoOff, 45)[0][10..14],
            [0xAC, 0, 1, 45]
        );
        let m = model(0x0577).unwrap();
        assert!(commands(&m, Control::Anc, 5).is_err());
        assert!(commands(&m, Control::AutoOff, 20).is_err());
        assert!(commands(&m, Control::EqPreset, 0).is_err());
        assert!(commands(&m, Control::Sidetone, 16).is_err());
    }

    #[test]
    fn v2_pro_2023_frames() {
        let f = frames(0x0555, Control::Sidetone, 7);
        assert_eq!(
            f[0][..14],
            [2, 0x80, 9, 0, 0, b'P', b'A', 8, 0, 4, 0x98, 0, 1, 1]
        );
        assert_eq!(
            f[1][..14],
            [2, 0x80, 9, 0, 0, b'P', b'A', 8, 0, 4, 0x99, 0, 1, 7]
        );
        assert_eq!(
            frames(0x0555, Control::AutoOff, 60)[0][9..14],
            [4, 0xAC, 0, 1, 60]
        );
        assert_eq!(
            pa_frame(0x21, &[])[..13],
            [2, 0x80, 8, 0, 0, b'P', b'A', 8, 0, 3, 0x21, 0, 0]
        );
        assert_eq!(
            pa_remote(true)[..12],
            [2, 0x80, 7, 0, 0, b'P', b'A', 0x0E, 0, 2, 0xE1, 1]
        );
        assert!(commands(&model(0x0555).unwrap(), Control::Sidetone, 11).is_err());
        let mut r = [0u8; 64];
        r[0] = 0x02;
        r[12..16].copy_from_slice(&[0x19, 0x01, 0x01, 0x05]);
        assert_eq!(pa_reply(0x19, &r), Some(5));
        assert_eq!(pa_reply(0x18, &r), None);
    }

    #[test]
    fn v3x_frames() {
        let f = frames(0x0A5B, Control::EqPreset, 1);
        assert_eq!(
            hex(&f[0]),
            "07000000000004180000010001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001c00"
        );
        let c = &commands(&model(0x057D).unwrap(), Control::Sidetone, 15).unwrap()[0];
        assert_eq!(
            hex(&v3x_frame(0x0A, c)),
            "07000a00000003180300030f00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001400"
        );
        let c = &commands(&model(0x057D).unwrap(), Control::AutoOff, 60).unwrap()[0];
        assert_eq!(
            hex(&v3x_frame(0x13, c)),
            "0700130000000207030e100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001800"
        );
        assert!(commands(&model(0x057D).unwrap(), Control::AutoOff, 0).is_err());
        let mut r = v3x_frame(0x41, c);
        r[1] = 0x02;
        assert_eq!(v3x_reply(0x41, c, &r), Some(true));
        assert_eq!(v3x_reply(0x42, c, &r), None);
        r[1] = 0x03;
        assert_eq!(v3x_reply(0x41, c, &r), Some(false));
    }
}
