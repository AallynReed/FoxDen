//! Ported from OpenRGB Controllers/SteelSeriesController (Rival 3, Aerox, Rival 5 controllers) (GPL-2.0-or-later).

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const BRIGHTNESS_MAX: u8 = 0x64;
const EFFECT_DIRECT: u8 = 0x04;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wireless {
    Aerox3,
    Aerox3Wired,
    Aerox3Gen2,
    Aerox5,
    Aerox5Wired,
    Aerox9,
    Aerox9Wired,
    Rival3,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Rival3,
    Aerox3,
    Aerox5,
    Wireless(Wireless),
    Rival5,
    PrimeMini,
    PrimeWired,
}

const fn det(name: &'static str, pid: u16, interface: i32, vendor_page: bool) -> Detector {
    Detector {
        name,
        vid: 0x1038,
        pid,
        interface: Some(interface),
        usage_page: if vendor_page { Some(0xFFC0) } else { None },
        usage: if vendor_page { Some(1) } else { None },
        open,
    }
}

static DETECTORS: [Detector; 24] = [
    det("SteelSeries Aerox 3 Wireless", 0x1838, 3, true),
    det("SteelSeries Aerox 3 Wireless Wired", 0x183A, 3, true),
    det(
        "SteelSeries Aerox 3 CS2 Dragon Lore Edition Wireless",
        0x1878,
        3,
        true,
    ),
    det(
        "SteelSeries Aerox 3 CS2 Dragon Lore Edition Wireless Wired",
        0x187A,
        3,
        true,
    ),
    det("SteelSeries Aerox 3 Wireless Gen 2", 0x1890, 3, true),
    det("SteelSeries Aerox 3 Wired", 0x1836, 3, true),
    det("SteelSeries Aerox 5 Wireless", 0x1852, 3, true),
    det("SteelSeries Aerox 5 Wireless Wired", 0x1854, 3, true),
    det(
        "SteelSeries Aerox 5 Destiny 2 Edition Wireless",
        0x185C,
        3,
        true,
    ),
    det(
        "SteelSeries Aerox 5 Destiny 2 Edition Wireless Wired",
        0x185E,
        3,
        true,
    ),
    det(
        "SteelSeries Aerox 5 Diablo IV Edition Wireless",
        0x1860,
        3,
        true,
    ),
    det(
        "SteelSeries Aerox 5 Diablo IV Edition Wireless Wired",
        0x1862,
        3,
        true,
    ),
    det("SteelSeries Aerox 5 Wired", 0x1850, 3, true),
    det("SteelSeries Aerox 9 Wireless", 0x1858, 3, true),
    det("SteelSeries Aerox 9 Wireless Wired", 0x185A, 3, true),
    det("SteelSeries Aerox 9 WOW Edition Wireless", 0x1876, 3, true),
    det(
        "SteelSeries Aerox 5 Diablo IV Edition Wireless",
        0x233A,
        3,
        true,
    ),
    det("SteelSeries Rival 3 (Old Firmware)", 0x1824, 3, false),
    det("SteelSeries Rival 3", 0x184C, 3, false),
    det("SteelSeries Rival 3 Wireless", 0x1830, 3, true),
    det("SteelSeries Rival 5", 0x183C, 0, true),
    det("SteelSeries Prime Mini", 0x184D, 3, false),
    det("SteelSeries Prime Wireless Wired", 0x1842, 3, false),
    det("SteelSeries Prime Mini Wireless Wired", 0x184A, 3, false),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

fn kind(pid: u16) -> Option<Kind> {
    use Wireless::*;
    Some(match pid {
        0x1824 | 0x184C => Kind::Rival3,
        0x1836 => Kind::Aerox3,
        0x1850 => Kind::Aerox5,
        0x183C => Kind::Rival5,
        0x1838 | 0x1878 => Kind::Wireless(Aerox3),
        0x183A | 0x187A => Kind::Wireless(Aerox3Wired),
        0x1890 => Kind::Wireless(Aerox3Gen2),
        0x1852 | 0x185C | 0x1860 | 0x233A => Kind::Wireless(Aerox5),
        0x1854 | 0x185E | 0x1862 => Kind::Wireless(Aerox5Wired),
        0x1858 | 0x1876 => Kind::Wireless(Aerox9),
        0x185A => Kind::Wireless(Aerox9Wired),
        0x1830 => Kind::Wireless(Rival3),
        0x184D => Kind::PrimeMini,
        0x1842 | 0x184A => Kind::PrimeWired,
        _ => return None,
    })
}

/// Zone ids from each controller's steelseries_mouse table.
fn leds(kind: Kind) -> &'static [u8] {
    match kind {
        Kind::Rival3 => &[1, 2, 3, 4],
        Kind::Aerox3 => &[0, 1, 2],
        Kind::Aerox5 => &[0, 1, 2, 3],
        Kind::Wireless(Wireless::Aerox9 | Wireless::Aerox9Wired) => &[0, 1, 2, 3],
        Kind::Wireless(Wireless::Rival3) => &[1],
        Kind::Wireless(_) => &[0, 1, 2],
        Kind::Rival5 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        Kind::PrimeMini | Kind::PrimeWired => &[0],
    }
}

fn padded(len: usize, head: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[..head.len()].copy_from_slice(head);
    v
}

fn is_wireless_link(w: Wireless) -> bool {
    matches!(
        w,
        Wireless::Aerox3 | Wireless::Aerox3Gen2 | Wireless::Aerox5 | Wireless::Aerox9
    )
}

/// SteelSeriesAeroxWirelessController::WriteBuffer: flag 0x40 and a read back over the dongle.
fn wireless_write(w: Wireless, mut buf: Vec<u8>) -> Vec<Op> {
    if is_wireless_link(w) {
        buf[1] |= 0x40;
        vec![Op::Write(buf), Op::Read(64, 250)]
    } else {
        vec![Op::Write(buf)]
    }
}

/// SetColor of the controller behind each kind; brightness is the Direct mode's 0x64.
fn set_color(kind: Kind, zone: u8, c: Color) -> Vec<Op> {
    match kind {
        Kind::Rival3 => vec![Op::Write(vec![
            0x00,
            0x05,
            0x00,
            zone,
            c.0,
            c.1,
            c.2,
            BRIGHTNESS_MAX,
        ])],
        Kind::Aerox3 => {
            let mut b = padded(65, &[0x00, 0x21, 1 << zone]);
            let at = 3 + zone as usize * 3;
            b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
            vec![Op::Write(b)]
        }
        Kind::Aerox5 => {
            let mut b = padded(65, &[0x00, 0x21, 0x01]);
            let at = match zone {
                0 => 3,
                1 => {
                    b[2] = 0x02;
                    6
                }
                2 => {
                    b[2] = 0x04;
                    9
                }
                3 => {
                    b[1] = 0x26;
                    4
                }
                _ => return Vec::new(),
            };
            b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
            vec![Op::Write(b), Op::Sleep(2)]
        }
        Kind::Wireless(w) => {
            let mut b = vec![0u8; 64];
            if w == Wireless::Rival3 {
                b[1..6].copy_from_slice(&[0x13, zone, c.0, c.1, c.2]);
            } else {
                b[1] = 0x21;
                b[2] = 0x01;
                if zone == 3 && matches!(w, Wireless::Aerox9 | Wireless::Aerox9Wired) {
                    b[3] = 0x00;
                    b[1] = 0x26;
                } else {
                    b[3] = zone;
                }
                b[4..7].copy_from_slice(&[c.0, c.1, c.2]);
            }
            wireless_write(w, b)
        }
        Kind::Rival5 => Vec::new(),
        Kind::PrimeMini => vec![Op::Write(padded(65, &[0x00, 0x21, 0x00, c.0, c.1, c.2]))],
        Kind::PrimeWired => vec![Op::Write(padded(
            64,
            &[0x00, 0x21, 0x01, 0x00, c.0, c.1, c.2],
        ))],
    }
}

fn set_brightness(kind: Kind) -> Vec<Op> {
    match kind {
        Kind::Aerox3 => vec![Op::Write(padded(65, &[0x00, 0x23, BRIGHTNESS_MAX]))],
        Kind::Aerox5 => vec![
            Op::Write(padded(65, &[0x00, 0x23, BRIGHTNESS_MAX])),
            Op::Sleep(2),
        ],
        Kind::Wireless(w) => wireless_write(w, padded(64, &[0x00, 0x23, BRIGHTNESS_MAX / 10])),
        _ => Vec::new(),
    }
}

/// SendInit from the constructor, or for the Rival 5 leaving Reactive and its first SetBrightness.
fn init(kind: Kind) -> Vec<Op> {
    match kind {
        Kind::Aerox3 | Kind::Aerox5 => vec![Op::Feature(padded(65, &[0x00, 0x2D]))],
        Kind::Wireless(_) => vec![Op::Feature(padded(64, &[0x00, 0x2D]))],
        Kind::Rival5 => vec![
            Op::Feature(padded(64, &[0x00, 0x26])),
            Op::Feature(padded(64, &[0x00, 0x23, 0x64])),
        ],
        Kind::Rival3 | Kind::PrimeMini | Kind::PrimeWired => Vec::new(),
    }
}

/// SteelSeriesRival5Controller::UpdateLEDs.
fn rival5_frame(colors: &[Color]) -> Vec<u8> {
    let mut b = padded(64, &[0x00, 0x21, 0xFF, 0x03]);
    for (i, c) in colors.iter().take(10).enumerate() {
        b[4 + i * 3..7 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

struct Mouse {
    kind: Kind,
    started: bool,
}

impl Proto for Mouse {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        let first = !self.started;
        if first {
            ops.extend(init(self.kind));
            self.started = true;
        }
        if self.kind == Kind::Rival5 {
            ops.push(Op::Feature(rival5_frame(colors)));
            return ops;
        }
        for (i, &zone) in leds(self.kind).iter().enumerate() {
            ops.extend(set_color(
                self.kind,
                zone,
                colors.get(i).copied().unwrap_or(Color::BLACK),
            ));
            if first && i == 0 {
                ops.extend(set_brightness(self.kind));
            }
        }
        if self.kind == Kind::Rival3 {
            ops.push(Op::Write(vec![0x00, 0x06, 0x00, EFFECT_DIRECT]));
        }
        ops
    }

    fn reset(&mut self) {
        self.started = false;
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let kind = kind(info.pid).ok_or("unknown SteelSeries mouse")?;
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("steelseries-mouse"),
        det.name,
        DeviceKind::Mouse,
        line(leds(kind).len()),
    );
    Ok(Driver::boxed(
        hid,
        "steelseries mouse",
        dev,
        Mouse {
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
        assert!(DETECTORS.iter().all(|d| kind(d.pid).is_some()));
        let r5 = DETECTORS.iter().find(|d| d.pid == 0x183C).unwrap();
        assert_eq!(
            (r5.interface, r5.usage_page, r5.usage),
            (Some(0), Some(0xFFC0), Some(1))
        );
        let r3 = DETECTORS.iter().find(|d| d.pid == 0x184C).unwrap();
        assert_eq!((r3.interface, r3.usage_page), (Some(3), None));
    }

    // RGBController_SteelSeriesRival3::DeviceUpdateLEDs on SteelSeriesRival3Controller.
    #[test]
    fn rival3_frame() {
        let mut m = Mouse {
            kind: Kind::Rival3,
            started: false,
        };
        let ops = m.frame(&[
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ]);
        let expect = vec![
            Op::Write(vec![0, 0x05, 0, 1, 1, 2, 3, 0x64]),
            Op::Write(vec![0, 0x05, 0, 2, 4, 5, 6, 0x64]),
            Op::Write(vec![0, 0x05, 0, 3, 7, 8, 9, 0x64]),
            Op::Write(vec![0, 0x05, 0, 4, 10, 11, 12, 0x64]),
            Op::Write(vec![0, 0x06, 0, 0x04]),
        ];
        assert_eq!(ops, expect);
    }

    // SteelSeriesAerox3Controller: SendInit, SetColor, then SetBrightness once.
    #[test]
    fn aerox3_frames() {
        let mut m = Mouse {
            kind: Kind::Aerox3,
            started: false,
        };
        let ops = m.frame(&[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)]);
        assert_eq!(ops.len(), 5);
        assert_eq!(ops[0], Op::Feature(padded(65, &[0, 0x2D])));
        assert_eq!(bytes(&ops[1])[..6], [0, 0x21, 0x01, 1, 2, 3]);
        assert_eq!(ops[2], Op::Write(padded(65, &[0, 0x23, 0x64])));
        let mid = bytes(&ops[3]);
        assert_eq!(mid.len(), 65);
        assert_eq!(mid[..9], [0, 0x21, 0x02, 0, 0, 0, 4, 5, 6]);
        assert_eq!(
            bytes(&ops[4])[..12],
            [0, 0x21, 0x04, 0, 0, 0, 0, 0, 0, 7, 8, 9]
        );
        assert_eq!(m.frame(&[Color::BLACK; 3]).len(), 3);
    }

    // SteelSeriesAerox5Controller::SetColor including the reactive zone.
    #[test]
    fn aerox5_reactive_zone() {
        let ops = set_color(Kind::Aerox5, 3, Color(1, 2, 3));
        assert_eq!(bytes(&ops[0])[..7], [0, 0x26, 0x01, 0, 1, 2, 3]);
        assert_eq!(ops[1], Op::Sleep(2));
        let ops = set_color(Kind::Aerox5, 2, Color(1, 2, 3));
        assert_eq!(
            bytes(&ops[0])[..12],
            [0, 0x21, 0x04, 0, 0, 0, 0, 0, 0, 1, 2, 3]
        );
    }

    // SteelSeriesAeroxWirelessController::SetColor/SetBrightness/WriteBuffer over the dongle.
    #[test]
    fn aerox9_wireless() {
        let ops = set_color(Kind::Wireless(Wireless::Aerox9), 3, Color(9, 8, 7));
        assert_eq!(bytes(&ops[0])[..7], [0, 0x66, 0x01, 0x00, 9, 8, 7]);
        assert_eq!(bytes(&ops[0]).len(), 64);
        assert_eq!(ops[1], Op::Read(64, 250));
        let ops = set_brightness(Kind::Wireless(Wireless::Aerox9));
        assert_eq!(bytes(&ops[0])[..3], [0, 0x63, 10]);
        let wired = set_color(Kind::Wireless(Wireless::Aerox5Wired), 1, Color(1, 1, 1));
        assert_eq!(wired.len(), 1);
        assert_eq!(bytes(&wired[0])[..7], [0, 0x21, 0x01, 0x01, 1, 1, 1]);
    }

    #[test]
    fn prime_colour_only() {
        let mut m = Mouse {
            kind: Kind::PrimeMini,
            started: false,
        };
        let ops = m.frame(&[Color(1, 2, 3)]);
        assert_eq!(ops, vec![Op::Write(padded(65, &[0, 0x21, 0x00, 1, 2, 3]))]);
        let mut m = Mouse {
            kind: Kind::PrimeWired,
            started: false,
        };
        let ops = m.frame(&[Color(4, 5, 6)]);
        assert_eq!(
            ops,
            vec![Op::Write(padded(64, &[0, 0x21, 0x01, 0x00, 4, 5, 6]))]
        );
        assert_eq!(DETECTORS.len(), 24);
    }

    // Rival 3 Wireless: 0x13 colour command, no flag, no effect packet.
    #[test]
    fn rival3_wireless() {
        let mut m = Mouse {
            kind: Kind::Wireless(Wireless::Rival3),
            started: false,
        };
        let ops = m.frame(&[Color(4, 5, 6)]);
        assert_eq!(ops[0], Op::Feature(padded(64, &[0, 0x2D])));
        assert_eq!(bytes(&ops[1])[..6], [0, 0x13, 1, 4, 5, 6]);
        assert_eq!(bytes(&ops[2])[..3], [0, 0x23, 10]);
        assert_eq!(ops.len(), 3);
    }

    // RGBController_SteelSeriesRival5: SetReactiveColor(false), SetBrightness(100), UpdateLEDs.
    #[test]
    fn rival5_frames() {
        let mut m = Mouse {
            kind: Kind::Rival5,
            started: false,
        };
        let colors: Vec<Color> = (0..10).map(|i| Color(i, i, i)).collect();
        let ops = m.frame(&colors);
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0], Op::Feature(padded(64, &[0, 0x26])));
        assert_eq!(ops[1], Op::Feature(padded(64, &[0, 0x23, 0x64])));
        let f = bytes(&ops[2]);
        assert_eq!(f[..7], [0, 0x21, 0xFF, 0x03, 0, 0, 0]);
        assert_eq!(f[31..34], [9, 9, 9]);
        assert!(f[34..].iter().all(|&b| b == 0));
        assert_eq!(m.frame(&colors).len(), 1);
    }
}
