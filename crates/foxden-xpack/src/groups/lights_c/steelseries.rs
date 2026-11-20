//! SteelSeries keyboards, mice, headsets and speakers (protocol learned from SignalRGB's Steelseries_*.js plugins).

use super::common::{open_with, positioned};
use crate::hid::{Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "steelseries-srgb";
const VID: u16 = 0x1038;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ApexM750Tkl,
    ApexProMini(u8),
    GameDac,
    ArctisPro,
    Arena,
    Rival3Gen2,
    Rival5Destiny,
    RivalPrime,
}

/// Lists LED cells row by row: `rows[y]` holds the x of each key.
fn rows(rows: &[&[u8]]) -> Vec<(u8, u8)> {
    rows.iter()
        .enumerate()
        .flat_map(|(y, xs)| xs.iter().map(move |&x| (x, y as u8)))
        .collect()
}

const M750_ROWS: [&[u8]; 6] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 15],
    &[0, 1, 2, 6, 10, 11, 12, 13, 14, 15, 16],
];

/// Colour slot of each M750 TKL key, in LED order.
const M750_SLOTS: [u8; 87] = [
    90, 91, 92, 93, 94, 96, 97, 98, 99, 101, 102, 103, 104, 105, 106, 107, //
    72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 86, 87, 88, 89, //
    54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 68, 69, 70, 71, //
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 49, //
    18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30, 34, //
    0, 1, 2, 4, 9, 10, 11, 12, 15, 16, 17,
];

const M750_LEN: usize = 513;
const M750_INIT: [&[u8]; 6] = [
    &[0, 0, 0, 0, 0x03, 0x00, 0x81],
    &[0, 0, 0, 0, 0x01, 0x00, 0x04, 0x01],
    &[0, 0, 0, 0, 0x05, 0x01, 0x01, 0x01, 0x00, 0xEB, 0x0C, 0x0C],
    &[0, 0, 0, 0, 0x04, 0x01, 0x00, 0x00, 0x00, 0x01],
    M750_APPLY,
    &[0, 0, 0, 0, 0x04, 0x01, 0x00, 0xFE, 0x00, 0x06],
];
const M750_APPLY: &[u8] = &[0, 0, 0, 0, 0x03, 0x01, 0x04, 0x00, 0x2D, 0x01];

/// M750 TKL colour feature report: header at byte 4, slot `s` at 9 + 3s.
fn m750_colors(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; M750_LEN];
    b[4..9].copy_from_slice(&[0xFF, 0x01, 0x03, 0x06, 0x12]);
    for (&s, &c) in M750_SLOTS.iter().zip(colors) {
        let at = 9 + s as usize * 3;
        b[at..at + 3].copy_from_slice(&rgb(c));
    }
    b
}

const MINI_ROWS: [&[u8]; 5] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13],
    &[0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13],
    &[0, 1, 2, 6, 10, 11, 12, 13],
];

/// HID key code of each Apex Pro Mini LED; the two ISO keys come first.
const MINI_KEYS: [u8; 63] = [
    100, 50, //
    41, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46, 42, //
    43, 20, 26, 8, 21, 23, 28, 24, 12, 18, 19, 47, 48, 49, //
    57, 4, 22, 7, 9, 10, 11, 13, 14, 15, 51, 52, 40, //
    225, 29, 27, 6, 25, 5, 17, 16, 54, 55, 56, 229, //
    224, 227, 226, 44, 230, 231, 240, 228,
];
const MINI_LEN: usize = 642;
const MINI_WIRELESS: u16 = 0x1626;

fn mini_positions() -> Vec<(u8, u8)> {
    let mut p = vec![(1, 4), (12, 3)];
    p.extend(rows(&MINI_ROWS));
    p
}

/// Apex Pro Mini feature report: header, key count, then key code and RGB per key.
fn mini_colors(header: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; MINI_LEN];
    b[1] = header;
    b[2] = MINI_KEYS.len() as u8;
    for (i, (&k, &c)) in MINI_KEYS.iter().zip(colors).enumerate() {
        let [r, g, bl] = rgb(c);
        b[3 + i * 4..7 + i * 4].copy_from_slice(&[k, r, g, bl]);
    }
    b
}

/// Headset LEDs: left can, right can, mic, muted mic.
const HEADSET_POS: [(u8, u8); 4] = [(0, 0), (2, 0), (2, 0), (1, 1)];

/// GameDAC zone colour (14-byte output report).
fn gamedac_zone(zone: u8, c: Color) -> [u8; 14] {
    let [r, g, b] = rgb(c);
    [
        0x00, 0xAA, zone, r, g, b, 0xFF, 0x32, 0xC8, 0xC8, 0x00, zone, 0x01, 0x00,
    ]
}
const GAMEDAC_COMMIT: [&[u8]; 4] = [
    &[0x00, 0xA5, 0x08, 0x0A, 0x00],
    &[0x00, 0xA5, 0x03, 0x0A, 0x00],
    &[0x00, 0xA5, 0x04, 0x0A, 0x00],
    &[0x00, 0xA3, 0x00],
];

/// Arctis Pro zone sequence: six 37-byte output reports with report id 0x06.
fn arctis_zone(zone: u8, c: Color) -> Vec<Vec<u8>> {
    let [r, g, b] = rgb(c);
    vec![
        vec![0x06, 0x81, 0x43, 0x01, 0x22],
        vec![0x06, 0x81, 0x43, 0x01, 0x23],
        vec![
            0x06, 0x8A, 0x42, 0x00, 0x20, 0x41, 0x00, r, g, b, 0xFF, 0x32, 0xC8, 0xC8,
        ],
        vec![0x06, 0x8A, 0x42, 0x00, 0x20, 0x41, 0x08, zone, 0x01],
        vec![0x06, 0x8A, 0x42, 0x00, 0x20, 0x60, zone],
        vec![0x06, 0x8A, 0x42, 0x20, 0x05],
    ]
}

/// Arena 7/9: left ring, left rear, right ring, right rear in one 64-byte report.
fn arena_colors(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0x06, 0xA7, 0x0F];
    b.extend(colors.iter().take(4).flat_map(|&c| rgb(c)));
    b
}

fn rival3_colors(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0x00, 0x21, 0x07];
    b.extend(colors.iter().take(3).flat_map(|&c| rgb(c)));
    b
}

fn rival5_colors(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0x00, 0x21, 0xFF, 0x03];
    b.extend(colors.iter().take(10).flat_map(|&c| rgb(c)));
    b
}

/// Rival Prime: the wheel colour twice, at bytes 3 and 18.
fn prime_colors(c: Color) -> Vec<u8> {
    let mut b = vec![0u8; 21];
    b[1..3].copy_from_slice(&[0x62, 0x01]);
    b[3..6].copy_from_slice(&rgb(c));
    b[18..21].copy_from_slice(&rgb(c));
    b
}

struct Ss(Kind);

impl Ss {
    fn send(&self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        match self.0 {
            Kind::ApexM750Tkl => {
                io.feature(&m750_colors(colors), M750_LEN)?;
                io.feature(M750_APPLY, M750_LEN)?;
                io.sleep(2);
            }
            Kind::ApexProMini(h) => io.feature(&mini_colors(h, colors), MINI_LEN)?,
            Kind::GameDac => {
                for (z, &c) in colors.iter().take(4).enumerate() {
                    io.write(&gamedac_zone(z as u8, c), 14)?;
                    io.sleep(20);
                }
                for p in GAMEDAC_COMMIT {
                    io.write(p, p.len())?;
                }
                io.sleep(150);
            }
            Kind::ArctisPro => {
                for (z, &c) in colors.iter().take(4).enumerate() {
                    for p in arctis_zone(z as u8, c) {
                        io.write(&p, 37)?;
                    }
                }
            }
            Kind::Arena => io.write(&arena_colors(colors), 64)?,
            Kind::Rival3Gen2 => io.write(&rival3_colors(colors), 65)?,
            Kind::Rival5Destiny => io.write(&rival5_colors(colors), 65)?,
            Kind::RivalPrime => {
                let c = colors.first().copied().unwrap_or(Color::BLACK);
                io.write(&prime_colors(c), 643)?;
            }
        }
        Ok(())
    }
}

impl Proto for Ss {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        match self.0 {
            Kind::ApexM750Tkl => {
                for p in M750_INIT {
                    io.feature(p, M750_LEN)?;
                }
            }
            Kind::GameDac => io.write(&[0x00, 0x2C, 0x01, 0x00], 4)?,
            Kind::Rival5Destiny => io.write(&[0x00, 0x09], 65)?,
            _ => {}
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.send(io, colors)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = match self.0 {
            Kind::GameDac => io.write(&[0x00, 0x2C, 0x00, 0x00], 4),
            _ => self.send(io, &[Color::BLACK; 87]),
        };
    }
}

fn kind(pid: u16) -> Option<Kind> {
    Some(match pid {
        0x0617 | 0x0619 => Kind::ApexM750Tkl,
        MINI_WIRELESS => Kind::ApexProMini(0x21),
        0x161E => Kind::ApexProMini(0x40),
        0x1280 => Kind::GameDac,
        0x1252 => Kind::ArctisPro,
        0x1A00 | 0x1A05 => Kind::Arena,
        0x1870 => Kind::Rival3Gen2,
        0x183E => Kind::Rival5Destiny,
        0x182E | 0x182A | 0x1856 | 0x182C => Kind::RivalPrime,
        _ => return None,
    })
}

/// Device layout: SignalRGB `Size()` and `LedPositions()` per kind.
fn layout(k: Kind) -> (DeviceKind, u32, u32, Vec<(u8, u8)>) {
    match k {
        Kind::ApexM750Tkl => (DeviceKind::Keyboard, 17, 6, rows(&M750_ROWS)),
        Kind::ApexProMini(_) => (DeviceKind::Keyboard, 14, 5, mini_positions()),
        Kind::GameDac | Kind::ArctisPro => (DeviceKind::Other, 3, 2, HEADSET_POS.to_vec()),
        Kind::Arena => (
            DeviceKind::Other,
            7,
            3,
            vec![(0, 0), (0, 2), (6, 0), (6, 2)],
        ),
        Kind::Rival3Gen2 => (DeviceKind::Mouse, 3, 3, vec![(2, 0), (2, 1), (2, 2)]),
        Kind::Rival5Destiny => (
            DeviceKind::Mouse,
            7,
            7,
            vec![
                (3, 0),
                (1, 2),
                (5, 2),
                (2, 3),
                (4, 3),
                (1, 4),
                (5, 4),
                (0, 5),
                (6, 5),
                (3, 6),
            ],
        ),
        Kind::RivalPrime => (DeviceKind::Mouse, 4, 4, vec![(2, 0)]),
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let k = kind(info.pid).ok_or("unknown SteelSeries device")?;
    let (dk, w, h, pos) = layout(k);
    let dev = positioned(&info.device_id(FAMILY), d.name, dk, w, h, &pos);
    open_with(api, info, FAMILY, dev, Ss(k))
}

const fn det(name: &'static str, pid: u16, interface: i32, page: Option<u16>) -> Detector {
    Detector {
        name,
        vid: VID,
        pid,
        interface: Some(interface),
        usage_page: page,
        usage: None,
        open,
    }
}

pub static DETECTORS: &[Detector] = &[
    det("SteelSeries Apex M750 TKL", 0x0617, 2, None),
    det("SteelSeries Apex M750 TKL PUBG Edition", 0x0619, 2, None),
    det("SteelSeries Apex Pro Mini Wireless", MINI_WIRELESS, 3, None),
    det("SteelSeries Apex Pro Mini", 0x161E, 1, None),
    det("SteelSeries Arctis Pro GameDAC", 0x1280, 0, None),
    det("SteelSeries Arctis Pro", 0x1252, 5, None),
    det("SteelSeries Arena 7", 0x1A00, 4, Some(0xFFC0)),
    det("SteelSeries Arena 9", 0x1A05, 4, Some(0xFFC0)),
    det("SteelSeries Rival 3 Gen 2", 0x1870, 3, None),
    det("SteelSeries Rival 5 Destiny", 0x183E, 0, None),
    det("SteelSeries Rival Prime", 0x182E, 0, None),
    det(
        "SteelSeries Prime Rainbow 6 Siege Black Ice Edition",
        0x182A,
        0,
        None,
    ),
    det("SteelSeries Prime CS:GO Neo Noir Edition", 0x1856, 0, None),
    det("SteelSeries Prime+", 0x182C, 0, None),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;
    use crate::hid::padded;

    const C: Color = Color(0x11, 0x22, 0x33);

    fn run(k: Kind, n: usize) -> Vec<Op> {
        let mut rec = Rec::default();
        let mut p = Ss(k);
        p.start(&mut rec).unwrap();
        p.frame(&mut rec, &vec![C; n]).unwrap();
        rec.ops
    }

    #[test]
    fn layouts_match_their_tables() {
        assert_eq!(rows(&M750_ROWS).len(), M750_SLOTS.len());
        assert_eq!(mini_positions().len(), MINI_KEYS.len());
        assert_eq!(DETECTORS.len(), 14);
        for d in DETECTORS {
            let k = kind(d.pid).unwrap();
            let (_, w, h, pos) = layout(k);
            assert!(
                pos.iter().all(|&(x, y)| (x as u32) < w && (y as u32) < h),
                "{}",
                d.name
            );
        }
    }

    #[test]
    fn m750_init_then_slotted_colors_and_apply() {
        let ops = run(Kind::ApexM750Tkl, 87);
        assert_eq!(ops.len(), 8);
        assert_eq!(ops[0], Op::Feature(padded(&[0, 0, 0, 0, 3, 0, 0x81], 513)));
        let Op::Feature(b) = &ops[6] else { panic!() };
        assert_eq!(b[4..9], [0xFF, 1, 3, 6, 0x12]);
        assert_eq!(b[9 + 90 * 3..9 + 90 * 3 + 3], [0x11, 0x22, 0x33]);
        assert_eq!(b[9 + 95 * 3], 0);
        assert_eq!(ops[7], Op::Feature(padded(M750_APPLY, 513)));
    }

    #[test]
    fn apex_pro_mini_header_by_pid() {
        let ops = run(Kind::ApexProMini(0x21), 63);
        let Op::Feature(b) = &ops[0] else { panic!() };
        assert_eq!(b.len(), 642);
        assert_eq!(b[..7], [0, 0x21, 63, 100, 0x11, 0x22, 0x33]);
        assert_eq!(b[3 + 62 * 4], 228);
        assert_eq!(kind(0x161E), Some(Kind::ApexProMini(0x40)));
    }

    #[test]
    fn gamedac_zones_then_commit() {
        let ops = run(Kind::GameDac, 4);
        assert_eq!(ops[0], Op::Write(vec![0, 0x2C, 1, 0]));
        assert_eq!(
            ops[2],
            Op::Write(vec![
                0, 0xAA, 1, 0x11, 0x22, 0x33, 0xFF, 0x32, 0xC8, 0xC8, 0, 1, 1, 0
            ])
        );
        assert_eq!(ops.len(), 1 + 4 + 4);
        assert_eq!(ops[8], Op::Write(vec![0, 0xA3, 0]));
        let mut rec = Rec::default();
        Ss(Kind::GameDac).stop(&mut rec);
        assert_eq!(rec.ops, vec![Op::Write(vec![0, 0x2C, 0, 0])]);
    }

    #[test]
    fn arctis_pro_six_reports_per_zone() {
        let ops = run(Kind::ArctisPro, 4);
        assert_eq!(ops.len(), 24);
        assert_eq!(
            ops[8],
            Op::Write(padded(
                &[
                    6, 0x8A, 0x42, 0, 0x20, 0x41, 0, 0x11, 0x22, 0x33, 0xFF, 0x32, 0xC8, 0xC8
                ],
                37
            ))
        );
        assert_eq!(
            ops[9],
            Op::Write(padded(&[6, 0x8A, 0x42, 0, 0x20, 0x41, 8, 1, 1], 37))
        );
        assert_eq!(ops[11], Op::Write(padded(&[6, 0x8A, 0x42, 0x20, 0x05], 37)));
    }

    #[test]
    fn mice_and_speakers() {
        let a = run(Kind::Arena, 4);
        let Op::Write(b) = &a[0] else { panic!() };
        assert_eq!(
            (b.len(), b[..6].to_vec(), b[14], b[15]),
            (64, vec![6, 0xA7, 0x0F, 0x11, 0x22, 0x33], 0x33, 0)
        );
        let r3 = run(Kind::Rival3Gen2, 3);
        assert_eq!(
            r3,
            vec![Op::Write(padded(
                &[
                    0, 0x21, 7, 0x11, 0x22, 0x33, 0x11, 0x22, 0x33, 0x11, 0x22, 0x33
                ],
                65
            ))]
        );
        let r5 = run(Kind::Rival5Destiny, 10);
        assert_eq!(r5[0], Op::Write(padded(&[0, 0x09], 65)));
        let Op::Write(b) = &r5[1] else { panic!() };
        assert_eq!(
            (b[..5].to_vec(), b[33], b[34]),
            (vec![0, 0x21, 0xFF, 3, 0x11], 0x33, 0)
        );
        let p = run(Kind::RivalPrime, 1);
        let Op::Write(b) = &p[0] else { panic!() };
        assert_eq!(
            (b.len(), b[..6].to_vec(), b[18..21].to_vec()),
            (
                643,
                vec![0, 0x62, 1, 0x11, 0x22, 0x33],
                vec![0x11, 0x22, 0x33]
            )
        );
    }
}
