//! JBL Quantum wireless headsets: two-byte feature reports `[id, value]` on the dongle's vendor
//! collection. Each setting is written to one id and read back from the id below it.

use foxden_battery::hid::{Coll, Dev};

use crate::Reading;
use crate::proto::{Control, Family, Model, Shape, Status};

pub const VID: u16 = 0x0ECB;
const BATTERY: u8 = 0x49;
const DIAL: u8 = 0x62;
const LINK: u8 = 0x68;
const DIAL_MAX: u8 = 16;

struct Kit {
    pids: &'static [u16],
    name: &'static str,
    anc: bool,
    /// The dial's direction is known: 0 is all chat.
    dial: bool,
}

const KITS: &[Kit] = &[
    Kit {
        pids: &[0x2069],
        name: "Quantum 810 Wireless",
        anc: true,
        dial: true,
    },
    Kit {
        pids: &[0x1F47, 0x203E],
        name: "Quantum 800",
        anc: true,
        dial: false,
    },
    Kit {
        pids: &[0x2088, 0x209C],
        name: "Quantum 910 Wireless",
        anc: true,
        dial: true,
    },
    Kit {
        pids: &[0x20A2, 0x20A4],
        name: "Quantum 910P Wireless",
        anc: true,
        dial: true,
    },
    Kit {
        pids: &[0x209D, 0x209F, 0x2147, 0x214A],
        name: "Quantum 910X Wireless",
        anc: true,
        dial: true,
    },
    Kit {
        pids: &[0x205C],
        name: "Quantum 610 Wireless",
        anc: false,
        dial: false,
    },
];

fn kit(pid: u16) -> Option<&'static Kit> {
    KITS.iter().find(|k| k.pids.contains(&pid))
}

pub fn model(pid: u16) -> Option<Model> {
    kit(pid).map(|k| Model {
        vid: VID,
        pid,
        name: k.name,
        family: Family::Jbl,
        sidetone: true,
        sidetone_max: 3,
        mic: false,
        anc: k.anc,
        wireless: true,
    })
}

pub fn fits(c: &Coll) -> bool {
    c.page == 0xFF13 && c.usage == 0x0001 && c.feat_len >= 2
}

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let mut out = vec![(
        Control::Sidetone,
        Shape::Choice {
            options: vec![(0, "Off"), (1, "Low"), (2, "Medium"), (3, "High")],
        },
    )];
    if m.anc {
        out.push((
            Control::Anc,
            Shape::Choice {
                options: vec![(0, "Off"), (1, "Noise cancelling"), (2, "TalkThru")],
            },
        ));
    }
    out
}

/// The ids `c` is written to and read from. These are the only ids FoxDen ever writes here.
fn ids(c: Control) -> Option<(u8, u8)> {
    match c {
        Control::Sidetone => Some((0x5D, 0x5C)),
        Control::Anc => Some((0x46, 0x45)),
        _ => None,
    }
}

fn shape(m: &Model, c: Control) -> Option<Shape> {
    controls(m)
        .into_iter()
        .find(|(k, _)| *k == c)
        .map(|(_, s)| s)
}

/// The report that sets `c` to `v`, given what its read-back id answered. A model without the
/// setting answers for a nearby id instead, so that refuses too.
fn frame(m: &Model, c: Control, v: i32, now: Option<u8>) -> Result<[u8; 2], String> {
    let s = shape(m, c).ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    let (set, _) = ids(c).ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    if !s.accepts(v) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    if !now.is_some_and(|b| s.accepts(b as i32)) {
        return Err("the headset's reply was not understood; nothing was changed".into());
    }
    Ok([set, v as u8])
}

/// Game and chat as the mixer takes them: both full at the centre.
fn mix(dial: u8) -> Option<(u8, u8)> {
    let half = |n: u8| (n as u16 * 200 / DIAL_MAX as u16).min(100) as u8;
    (dial <= DIAL_MAX).then(|| (half(dial), half(DIAL_MAX - dial)))
}

/// Byte 1 of feature `id`, only when the reply carries that id.
fn byte(f: &Coll, d: &Dev, id: u8) -> Option<u8> {
    foxden_battery::jbl_feature_byte(f, id).or_else(|| {
        d.get_feature(id)
            .filter(|b| b.len() >= 2 && b[0] == id)
            .map(|b| b[1])
    })
}

fn link(m: &Model, f: &Coll, d: &Dev) -> Option<Status> {
    if byte(f, d, LINK)? != 1 {
        return Some(Status::default());
    }
    let dial = kit(m.pid).is_some_and(|k| k.dial);
    Some(Status {
        connected: true,
        chatmix: dial.then(|| byte(f, d, DIAL).and_then(mix)).flatten(),
        ..Status::default()
    })
}

pub fn status(m: &Model, f: &Coll) -> Option<Status> {
    link(m, f, &Dev::open_feature_only(f)?)
}

pub fn read(m: &Model, f: &Coll) -> Option<Reading> {
    let d = Dev::open_feature_only(f)?;
    let mut status = link(m, f, &d)?;
    let mut values = Vec::new();
    if status.connected {
        status.battery = byte(f, &d, BATTERY).filter(|p| *p <= 100);
        for (c, s) in controls(m) {
            if let Some(v) = ids(c).and_then(|(_, get)| byte(f, &d, get))
                && s.accepts(v as i32)
            {
                values.push((c, v as i32));
            }
        }
    }
    Some(Reading {
        status: Some(status),
        values,
    })
}

pub fn set(m: &Model, f: &Coll, c: Control, v: i32) -> Result<(), String> {
    let (_, get) = ids(c)
        .filter(|_| shape(m, c).is_some())
        .ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    let d = Dev::open(f).ok_or("the headset's receiver could not be opened")?;
    if byte(f, &d, LINK) != Some(1) {
        return Err("the headset is switched off".into());
    }
    let report = frame(m, c, v, byte(f, &d, get))?;
    if d.set_feature(&report) {
        Ok(())
    } else {
        Err("the headset's receiver did not take the setting".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q810() -> Model {
        model(0x2069).unwrap()
    }

    #[test]
    fn frames() {
        let m = q810();
        assert_eq!(frame(&m, Control::Sidetone, 0, Some(2)).unwrap(), [0x5D, 0]);
        assert_eq!(frame(&m, Control::Sidetone, 3, Some(0)).unwrap(), [0x5D, 3]);
        assert_eq!(frame(&m, Control::Anc, 2, Some(1)).unwrap(), [0x46, 2]);
        assert_eq!(frame(&m, Control::Anc, 0, Some(0)).unwrap(), [0x46, 0]);
        assert!(frame(&m, Control::Sidetone, 4, Some(0)).is_err());
        assert!(frame(&m, Control::Anc, 3, Some(0)).is_err());
        assert!(frame(&m, Control::Sidetone, 1, None).is_err());
        assert!(frame(&m, Control::Sidetone, 1, Some(0x10)).is_err());
        assert!(frame(&m, Control::AutoOff, 6, Some(6)).is_err());
        assert!(frame(&m, Control::MicVolume, 1, Some(1)).is_err());
    }

    #[test]
    fn only_two_ids_are_ever_written() {
        let m = q810();
        for (c, _) in controls(&m) {
            for v in 0..=3 {
                if let Ok(r) = frame(&m, c, v, Some(0)) {
                    assert!(matches!(r[0], 0x46 | 0x5D));
                }
            }
        }
    }

    #[test]
    fn models() {
        let q610 = model(0x205C).unwrap();
        assert_eq!(
            controls(&q610).iter().map(|c| c.0).collect::<Vec<_>>(),
            [Control::Sidetone]
        );
        assert!(frame(&q610, Control::Anc, 1, Some(0)).is_err());
        assert_eq!(model(0x214A).unwrap().name, "Quantum 910X Wireless");
        assert!(model(0x2125).is_none());
        assert!(model(0x208A).is_none());
        assert!(kit(0x205C).is_some_and(|k| !k.dial));
    }

    #[test]
    fn dial() {
        assert_eq!(mix(8), Some((100, 100)));
        assert_eq!(mix(0), Some((0, 100)));
        assert_eq!(mix(16), Some((100, 0)));
        assert_eq!(mix(4), Some((50, 100)));
        assert_eq!(mix(17), None);
    }
}
