//! Ported from OpenRGB Controllers/SteelSeriesController/SteelSeriesRivalController and SteelSeriesSenseiController (GPL-2.0-or-later).

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Rival100,
    Rival300,
    Rival600,
    Rival650,
    Rival700,
    Sensei,
    SenseiRaw,
}

const fn det(name: &'static str, pid: u16) -> Detector {
    Detector {
        name,
        vid: 0x1038,
        pid,
        interface: Some(0),
        usage_page: None,
        usage: None,
        open,
    }
}

static DETECTORS: [Detector; 37] = [
    det("SteelSeries Rival 100", 0x1702),
    det("SteelSeries Rival 100 (Dell China)", 0x170A),
    det("SteelSeries Rival 100 Dota 2 Edition", 0x170B),
    det("SteelSeries Rival 100 DotA 2 Edition", 0x170C),
    det("SteelSeries Rival 105", 0x1814),
    det("SteelSeries Rival 106", 0x1816),
    det("SteelSeries Rival 110", 0x1729),
    det("SteelSeries Rival", 0x1384),
    det("SteelSeries Rival 300", 0x1710),
    det("SteelSeries Rival 300 Fallout 4 Edition", 0x1712),
    det("SteelSeries Rival 300 Evil Geniuses Edition", 0x171C),
    det("Acer Predator Gaming Mouse (Rival 300)", 0x1714),
    det("SteelSeries Rival 300 CS:GO Fade Edition", 0x1394),
    det("SteelSeries Rival 300 CS:GO Fade Edition (stm32)", 0x1716),
    det("SteelSeries Rival 300 CS:GO Hyperbeast Edition", 0x171A),
    det("SteelSeries Rival 300 Dota 2 Edition", 0x1392),
    det("SteelSeries Rival 300 HP Omen Edition", 0x1718),
    det("SteelSeries Rival 300 Black Ops Edition", 0x1710),
    det("SteelSeries Rival 310", 0x1720),
    det("SteelSeries Rival 310 CS:GO Howl Edition", 0x171E),
    det("SteelSeries Rival 310 PUBG Edition", 0x1736),
    det("SteelSeries Rival 600", 0x1724),
    det("SteelSeries Rival 600 Dota 2 Edition", 0x172E),
    det("SteelSeries Rival 650", 0x172B),
    det("SteelSeries Rival 650 Wireless", 0x1726),
    det("SteelSeries Rival 500", 0x170E),
    det("SteelSeries Rival 700", 0x1700),
    det("SteelSeries Rival 710", 0x1730),
    det("SteelSeries Sensei TEN", 0x1832),
    det("SteelSeries Sensei TEN CS:GO Neon Rider Edition", 0x1834),
    det("SteelSeries Sensei 310", 0x1722),
    det("SteelSeries Sensei [RAW]", 0x1369),
    det("SteelSeries Sensei [RAW] Diablo III Edition", 0x1362),
    det("SteelSeries Sensei [RAW] Guild Wars 2 Edition", 0x136D),
    det("SteelSeries Sensei [RAW] CoD Black Ops II Edition", 0x136F),
    det("SteelSeries Sensei [RAW] World of Tanks Edition", 0x1380),
    det(
        "SteelSeries Sensei [RAW] Heroes of the Storm Edition",
        0x1390,
    ),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

fn kind(pid: u16) -> Option<Kind> {
    Some(match pid {
        0x1702 | 0x170A | 0x170B | 0x170C | 0x1814 | 0x1816 | 0x1729 => Kind::Rival100,
        0x1384 | 0x1710 | 0x1712 | 0x171C | 0x1714 | 0x1394 | 0x1716 | 0x171A | 0x1392 | 0x1718 => {
            Kind::Rival300
        }
        0x1720 | 0x171E | 0x1736 | 0x1832 | 0x1834 | 0x1722 => Kind::Sensei,
        0x1724 | 0x172E => Kind::Rival600,
        0x172B | 0x1726 => Kind::Rival650,
        0x170E | 0x1700 | 0x1730 => Kind::Rival700,
        0x1369 | 0x1362 | 0x136D | 0x136F | 0x1380 | 0x1390 => Kind::SenseiRaw,
        _ => return None,
    })
}

/// led.value of each LED, as RGBController_SteelSeriesRival::SetupZones sets them.
fn leds(kind: Kind) -> &'static [u8] {
    match kind {
        Kind::Rival100 | Kind::SenseiRaw => &[0],
        Kind::Rival300 | Kind::Rival700 | Kind::Sensei => &[0, 1],
        Kind::Rival650 => &[0x11, 0x10, 0x12, 0x14, 0x16, 0x13, 0x15, 0x17],
        Kind::Rival600 => &[0x01, 0x00, 0x02, 0x04, 0x06, 0x03, 0x05, 0x07],
    }
}

/// send_usb_msg: report id 0 in front of the data.
fn msg(data: &[u8]) -> Op {
    let mut v = Vec::with_capacity(data.len() + 1);
    v.push(0x00);
    v.extend_from_slice(data);
    Op::Write(v)
}

/// SetLightEffectAll(STEELSERIES_RIVAL_EFFECT_DIRECT).
fn effect_direct(kind: Kind) -> Vec<Op> {
    let effect = |zone_byte: Option<u8>| {
        let mut b = [0u8; 9];
        if let Some(z) = zone_byte {
            b[0] = 0x07;
            b[1] = z;
        }
        b[2] = 0x01;
        msg(&b)
    };
    match kind {
        Kind::Rival100 => vec![effect(Some(0))],
        Kind::SenseiRaw => vec![msg(&[0x07, 0x01, 0x01, 0, 0, 0, 0, 0, 0])],
        Kind::Rival300 => vec![effect(Some(1)), effect(Some(2))],
        Kind::Rival650 => (0x10..0x18).map(|_| effect(None)).collect(),
        _ => Vec::new(),
    }
}

/// SteelSeriesRivalController::SetColor.
fn set_color(kind: Kind, zone: u8, c: Color) -> Vec<Op> {
    match kind {
        Kind::Rival100 | Kind::Rival300 => {
            let mut b = [0u8; 9];
            if kind == Kind::Rival100 {
                b[0] = 0x05;
            } else {
                b[0] = 0x08;
                b[1] = zone + 1;
            }
            b[2..5].copy_from_slice(&[c.0, c.1, c.2]);
            vec![msg(&b)]
        }
        Kind::SenseiRaw => vec![msg(&[0x05, 0x01, raw_level(c), 0, 0, 0, 0, 0, 0])],
        Kind::Rival600 => vec![Op::Write(vec![
            0x1C,
            0x27,
            0x00,
            1u8 << zone,
            c.0,
            c.1,
            c.2,
        ])],
        Kind::Rival650 => {
            let mut a = [0u8; 60];
            a[0x00] = 0x03;
            a[0x04] = 0x30;
            a[0x06] = 0x10;
            a[0x07] = 0x27;
            a[0x16] = 0x01;
            a[0x1E] = 0x04;
            a[0x1F..0x22].copy_from_slice(&[c.0, c.1, c.2]);
            a[0x22] = 0xFF;
            a[0x27] = 0xFF;
            a[0x29] = 0x54;
            a[0x2C] = 0xFF;
            a[0x2D] = 0x54;
            a[0x2E..0x31].copy_from_slice(&[c.0, c.1, c.2]);
            a[0x31] = 0x56;
            let mut b = [0u8; 60];
            b[0x00] = 0x03;
            b[0x02] = 0x30;
            b[0x04] = 0x2C;
            let mut d = [0u8; 60];
            d[0x00] = 0x05;
            d[0x02] = zone;
            d[0x03] = 0xFF;
            d[0x08] = 0x5C;
            let mut e = [0u8; 60];
            e[0x00] = 0x1C;
            e[0x02] = 0x55;
            e[0x04] = 0x46;
            vec![msg(&a), msg(&b), msg(&d), msg(&e)]
        }
        Kind::Rival700 => {
            let mut b = vec![0u8; 579];
            b[1] = 0x05;
            b[3] = zone;
            b[4..7].copy_from_slice(&[c.0, c.1, c.2]);
            b[12] = zone;
            b[13] = 0x01;
            vec![Op::Feature(b)]
        }
        Kind::Sensei => {
            let mut b = [0u8; 65];
            b[0x00] = 0x5B;
            b[0x02] = zone;
            b[0x13] = 0x01;
            b[0x1B] = 0x01;
            b[0x1C..0x1F].copy_from_slice(&[c.0, c.1, c.2]);
            b[0x1F..0x22].copy_from_slice(&[c.0, c.1, c.2]);
            vec![msg(&b)]
        }
    }
}

/// White LED levels: 1 off, then low, medium, high by the brightest channel.
fn raw_level(c: Color) -> u8 {
    match c.0.max(c.1).max(c.2) {
        0 => 1,
        1..=85 => 2,
        86..=170 => 3,
        _ => 4,
    }
}

struct Rival {
    kind: Kind,
    started: bool,
}

impl Proto for Rival {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        if !self.started {
            ops.extend(effect_direct(self.kind));
            self.started = true;
        }
        for (i, &zone) in leds(self.kind).iter().enumerate() {
            ops.extend(set_color(
                self.kind,
                zone,
                colors.get(i).copied().unwrap_or(Color::BLACK),
            ));
        }
        ops
    }

    fn reset(&mut self) {
        self.started = false;
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let kind = kind(info.pid).ok_or("unknown SteelSeries Rival")?;
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("steelseries-rival"),
        det.name,
        DeviceKind::Mouse,
        line(leds(kind).len()),
    );
    Ok(Driver::boxed(
        hid,
        "steelseries rival",
        dev,
        Rival {
            kind,
            started: false,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(op: &Op) -> &[u8] {
        match op {
            Op::Write(b) | Op::Feature(b) => b,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_detector_has_a_kind() {
        let real = detectors();
        assert_eq!(real.len(), 37);
        assert!(
            real.iter()
                .all(|d| kind(d.pid).is_some() && d.interface == Some(0))
        );
    }

    // RGBController_SteelSeriesRival::DeviceUpdateMode (Direct) then SetColor, Rival 300.
    #[test]
    fn rival300_first_frame() {
        let mut r = Rival {
            kind: Kind::Rival300,
            started: false,
        };
        let ops = r.frame(&[Color(1, 2, 3), Color(4, 5, 6)]);
        let expect: Vec<Vec<u8>> = vec![
            vec![0, 0x07, 0x01, 0x01, 0, 0, 0, 0, 0, 0],
            vec![0, 0x07, 0x02, 0x01, 0, 0, 0, 0, 0, 0],
            vec![0, 0x08, 0x01, 1, 2, 3, 0, 0, 0, 0],
            vec![0, 0x08, 0x02, 4, 5, 6, 0, 0, 0, 0],
        ];
        assert_eq!(ops, expect.into_iter().map(Op::Write).collect::<Vec<_>>());
        assert_eq!(r.frame(&[Color(1, 2, 3), Color(4, 5, 6)]).len(), 2);
    }

    // SteelSeriesRivalController::SetLightEffect + SetColor for the Rival 100.
    #[test]
    fn rival100_frame() {
        let mut r = Rival {
            kind: Kind::Rival100,
            started: false,
        };
        let ops = r.frame(&[Color(9, 8, 7)]);
        assert_eq!(bytes(&ops[0]), &[0, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        assert_eq!(bytes(&ops[1]), &[0, 0x05, 0x00, 9, 8, 7, 0, 0, 0, 0]);
    }

    #[test]
    fn sensei_raw_steady_then_level() {
        let mut r = Rival {
            kind: Kind::SenseiRaw,
            started: false,
        };
        let ops = r.frame(&[Color(200, 0, 0)]);
        assert_eq!(bytes(&ops[0]), &[0, 0x07, 0x01, 0x01, 0, 0, 0, 0, 0, 0]);
        assert_eq!(bytes(&ops[1]), &[0, 0x05, 0x01, 0x04, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            r.frame(&[Color::BLACK]),
            vec![msg(&[0x05, 0x01, 0x01, 0, 0, 0, 0, 0, 0])]
        );
        assert_eq!(raw_level(Color(0, 90, 10)), 3);
    }

    // SetRival600Color: 7-byte write with 0x1C in the report id slot.
    #[test]
    fn rival600_colour() {
        let ops = set_color(Kind::Rival600, 0x07, Color(1, 2, 3));
        assert_eq!(ops, vec![Op::Write(vec![0x1C, 0x27, 0x00, 0x80, 1, 2, 3])]);
        assert!(effect_direct(Kind::Rival600).is_empty());
    }

    // SetRival650Color: four 61-byte writes; SetLightEffectAll writes eight effect packets.
    #[test]
    fn rival650_colour() {
        let ops = set_color(Kind::Rival650, 0x12, Color(10, 20, 30));
        assert_eq!(ops.len(), 4);
        let a = bytes(&ops[0]);
        assert_eq!(a.len(), 61);
        assert_eq!(a[..9], [0, 0x03, 0, 0, 0, 0x30, 0, 0x10, 0x27]);
        assert_eq!(a[0x17], 0x01);
        assert_eq!(a[0x1F..0x24], [0x04, 10, 20, 30, 0xFF]);
        assert_eq!(a[0x28], 0xFF);
        assert_eq!(a[0x2A], 0x54);
        assert_eq!(a[0x2D..0x33], [0xFF, 0x54, 10, 20, 30, 0x56]);
        assert_eq!(bytes(&ops[1])[..6], [0, 0x03, 0, 0x30, 0, 0x2C]);
        assert_eq!(
            bytes(&ops[2])[..10],
            [0, 0x05, 0, 0x12, 0xFF, 0, 0, 0, 0, 0x5C]
        );
        assert_eq!(bytes(&ops[3])[..6], [0, 0x1C, 0, 0x55, 0, 0x46]);
        let fx = effect_direct(Kind::Rival650);
        assert_eq!(fx.len(), 8);
        assert_eq!(bytes(&fx[0]), &[0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0]);
    }

    // SetRival700Color: 579-byte feature report.
    #[test]
    fn rival700_colour() {
        let ops = set_color(Kind::Rival700, 1, Color(5, 6, 7));
        let b = bytes(&ops[0]);
        assert_eq!(b.len(), 579);
        assert_eq!(b[..7], [0, 0x05, 0, 1, 5, 6, 7]);
        assert_eq!(b[12..14], [1, 0x01]);
        assert!(b[14..].iter().all(|&x| x == 0));
    }

    // SteelSeriesSenseiController::SetColor.
    #[test]
    fn sensei_colour() {
        let mut s = Rival {
            kind: Kind::Sensei,
            started: false,
        };
        let ops = s.frame(&[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(ops.len(), 2);
        let b = bytes(&ops[1]);
        assert_eq!(b.len(), 66);
        assert_eq!(b[..4], [0, 0x5B, 0, 1]);
        assert_eq!(b[0x14], 0x01);
        assert_eq!(b[0x1C..0x23], [0x01, 4, 5, 6, 4, 5, 6]);
    }
}
