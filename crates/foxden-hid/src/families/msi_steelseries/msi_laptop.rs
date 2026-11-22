// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, matrix, rows};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const VID: u16 = 0x1038;
const A18_KLC: u16 = 0x1122;
const A18_ALC: u16 = 0x1161;
const GE68HX_KEYBOARD: u16 = 0x113a;
const GE68HX_LIGHTBAR: u16 = 0x114b;
const GE66_11UH_LIGHTBAR: u16 = 0x1132;
const RAIDER_GE66_LIGHTBAR: u16 = 0x113c;
const GE76_11UH_LIGHTBAR: u16 = 0x113e;
const GE78HX_LIGHTBAR: u16 = 0x114d;
const TITAN18HX_ALC: u16 = 0x115b;
const NA: u8 = 0xFF;

const fn det(name: &'static str, pid: u16) -> Detector {
    Detector {
        name,
        vid: VID,
        pid,
        interface: Some(0),
        usage_page: None,
        usage: None,
        open,
    }
}

static DETECTORS: [Detector; 9] = [
    det("MSI Laptop Keyboard", A18_KLC),
    det("MSI Laptop Lightbar", A18_ALC),
    det("MSI GE68HX Keyboard", GE68HX_KEYBOARD),
    det("MSI GE68HX Lightbar", GE68HX_LIGHTBAR),
    det("MSI GE66 11UH Lightbar", GE66_11UH_LIGHTBAR),
    det("MSI Raider GE66 Lightbar", RAIDER_GE66_LIGHTBAR),
    det("MSI GE76 Raider 11UH Lightbar", GE76_11UH_LIGHTBAR),
    det("MSI Raider GE78 HX Lightbar", GE78HX_LIGHTBAR),
    det("MSI Titan 18 HX Touchpad and Logo", TITAN18HX_ALC),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Klc,
    Alc,
    /// ALC zones with the unused records left zero.
    AlcZeroFill,
    KlcLightbar,
    /// KLC lightbar records, each report applied on its own after a short wait.
    AlcLightbar,
    StreamLightbar,
}

/// SetLEDs for the keyboard (KLC) and the A18 lightbar (ALC): one 525-byte feature report.
fn zone_packet(kind: Kind, ids: &[u8], colors: &[Color]) -> Vec<u8> {
    let mut buf = vec![0u8; 525];
    buf[1] = 0x0C;
    buf[3] = if kind == Kind::Klc { 0x66 } else { 0x06 };
    if kind != Kind::AlcZeroFill {
        for i in 0..(525 - 5) / 4 {
            buf[5 + i * 4] = 0xFF;
        }
    }
    for (i, (&id, c)) in ids.iter().zip(colors).enumerate() {
        let at = 5 + i * 4;
        if at + 3 >= buf.len() {
            break;
        }
        buf[at..at + 4].copy_from_slice(&[id, c.0, c.1, c.2]);
    }
    buf
}

/// SetLEDs for the GE68HX lightbar: 30 LEDs per feature report, then a commit written as output.
fn lightbar_packets(ids: &[u8], colors: &[Color]) -> Vec<Op> {
    lightbar_ops(ids, colors, false)
}

fn lightbar_ops(ids: &[u8], colors: &[Color], commit_each: bool) -> Vec<Op> {
    let n = ids.len().min(colors.len());
    let mut ops = Vec::new();
    let mut sent = 0;
    while sent < n {
        let count = (n - sent).min(30);
        let mut buf = vec![0u8; 525];
        buf[1] = 0x0E;
        buf[3] = count as u8;
        for i in 0..count {
            let at = 5 + i * 12;
            let c = colors[sent + i];
            buf[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
            buf[at + 9] = 0x01;
            buf[at + 11] = ids[sent + i];
        }
        ops.push(Op::Feature(buf));
        sent += count;
        if commit_each {
            ops.push(Op::Sleep(25));
            ops.push(Op::Write(commit()));
        }
    }
    if !commit_each {
        ops.push(Op::Write(commit()));
    }
    ops
}

fn commit() -> Vec<u8> {
    let mut commit = vec![0u8; 65];
    commit[1] = 0x0D;
    commit[3] = 0x02;
    commit
}

/// GE66 11UH lightbar stream frame: one 0x0C report of [id, r, g, b].
fn stream_packet(ids: &[u8], colors: &[Color]) -> Vec<u8> {
    let mut buf = vec![0u8; 525];
    buf[1] = 0x0C;
    buf[3] = ids.len() as u8;
    for (i, (&id, c)) in ids.iter().zip(colors).enumerate() {
        buf[5 + i * 4..9 + i * 4].copy_from_slice(&[id, c.0, c.1, c.2]);
    }
    buf
}

/// setSoftwareLightingMode (0x51 reset, 0x10 ping) and the one 0x0D apply that leaves the onboard effect.
fn stream_start() -> Vec<Op> {
    let w = |head: &[u8]| {
        let mut b = vec![0u8; 65];
        b[..head.len()].copy_from_slice(head);
        Op::Write(b)
    };
    vec![
        w(&[0x00, 0x51, 0x00, 0xFF, 0xFF, 0x99]),
        w(&[0x00, 0x10]),
        w(&[0x00, 0x0D, 0x00, 0x02]),
    ]
}

struct Laptop {
    kind: Kind,
    ids: Vec<u8>,
    started: bool,
}

impl Proto for Laptop {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        match self.kind {
            Kind::KlcLightbar => lightbar_packets(&self.ids, colors),
            Kind::AlcLightbar => lightbar_ops(&self.ids, colors, true),
            Kind::StreamLightbar => {
                let mut ops = if self.started {
                    Vec::new()
                } else {
                    stream_start()
                };
                self.started = true;
                ops.push(Op::Feature(stream_packet(&self.ids, colors)));
                ops
            }
            kind => vec![Op::Feature(zone_packet(kind, &self.ids, colors))],
        }
    }

    fn reset(&mut self) {
        self.started = false;
    }
}

fn klc_map() -> Vec<Option<usize>> {
    KLC_MAP
        .iter()
        .map(|&v| (v != NA).then_some(v as usize))
        .collect()
}

/// The model is picked by SMBIOS; every listed model shares the keyboard, and each lightbar PID belongs to one model.
fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let (kind, ids, dev_kind, layout) = match info.pid {
        A18_KLC | GE68HX_KEYBOARD => (
            Kind::Klc,
            KLC_IDS.to_vec(),
            DeviceKind::Keyboard,
            matrix(KLC_IDS.len(), 6, 23, &klc_map()),
        ),
        A18_ALC => (
            Kind::Alc,
            A18_ALC_IDS.to_vec(),
            DeviceKind::Strip,
            rows(&[3, 1]),
        ),
        GE68HX_LIGHTBAR => (
            Kind::KlcLightbar,
            (0..=0x34).collect(),
            DeviceKind::Strip,
            rows(&[1, 52]),
        ),
        RAIDER_GE66_LIGHTBAR => (
            Kind::KlcLightbar,
            (0..35).collect(),
            DeviceKind::Strip,
            rows(&[35]),
        ),
        GE76_11UH_LIGHTBAR => (
            Kind::AlcLightbar,
            (0..30).collect(),
            DeviceKind::Strip,
            rows(&[30]),
        ),
        GE78HX_LIGHTBAR => (
            Kind::AlcLightbar,
            (0..55).collect(),
            DeviceKind::Strip,
            rows(&[1, 54]),
        ),
        TITAN18HX_ALC => (
            Kind::AlcZeroFill,
            (0..6).collect(),
            DeviceKind::Strip,
            rows(&[5, 1]),
        ),
        GE66_11UH_LIGHTBAR => (
            Kind::StreamLightbar,
            (0..35).collect(),
            DeviceKind::Strip,
            rows(&[35]),
        ),
        _ => return Err("unknown MSI laptop device".into()),
    };
    let hid = info.open(api)?;
    let dev = device(info.device_id("msi-laptop"), det.name, dev_kind, layout);
    Ok(Driver::boxed(
        hid,
        "msi laptop",
        dev,
        Laptop {
            kind,
            ids,
            started: false,
        },
    ))
}

const KLC_IDS: [u8; 102] = [
    0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13,
    0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23,
    0x24, 0x25, 0x26, 0x27, 0x29, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x33, 0x34, 0x35, 0x36, 0x37,
    0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
    0x49, 0x4B, 0x4C, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A,
    0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x61, 0x62, 0x63, 0x66, 0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xF0,
    0x28, 0x2A, 0x31, 0x64, 0xE5, 0xE6,
];

#[rustfmt::skip]
const KLC_MAP: [u8; 138] = [
    36, NA, 50, 51, 52, 53, NA, 54, 55, 56, 57, NA, 58, 59, 60, 61, 62, 63, NA, NA, NA, NA, 89,
    45, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 39, 40, 97, NA, 64, 65, NA, 72, 73, 74, 75, NA,
    37, 16, 22, 4, 17, 19, 24, 20, 8, 14, 15, 41, 42, 98, NA, 66, 67, NA, 84, 85, 86, 76, NA,
    49, 0, 18, 3, 5, 6, 7, 9, 10, 11, 43, 44, 96, NA, NA, NA, NA, NA, 81, 82, 83, NA, NA,
    91, 25, 23, 2, 21, 1, 13, 12, 46, 47, 48, 100, NA, NA, NA, NA, 71, NA, 78, 79, 80, 77, NA,
    90, 93, 92, NA, NA, NA, 38, NA, NA, NA, 101, 94, 95, NA, 69, 70, 68, NA, 87, NA, 88, NA, NA,
];

const A18_ALC_IDS: [u8; 4] = [0x00, 0x01, 0x02, 0x03];

#[cfg(test)]
mod tests {
    use super::*;

    // MSILaptopController::SetLEDs, KLC branch.
    #[test]
    fn keyboard_packet() {
        let mut colors = vec![Color::BLACK; 102];
        colors[0] = Color(1, 2, 3);
        colors[101] = Color(7, 8, 9);
        let p = zone_packet(Kind::Klc, &KLC_IDS, &colors);
        assert_eq!(p.len(), 525);
        assert_eq!(p[..5], [0x00, 0x0C, 0x00, 0x66, 0x00]);
        assert_eq!(p[5..9], [0x04, 1, 2, 3]);
        assert_eq!(p[5 + 101 * 4..5 + 102 * 4], [0xE6, 7, 8, 9]);
        assert_eq!(p[5 + 102 * 4..5 + 102 * 4 + 4], [0xFF, 0, 0, 0]);
        assert_eq!(p[5 + 129 * 4], 0xFF);
        assert_eq!(p[524], 0);
    }

    // MSILaptopController::SetLEDs, ALC branch with the A18 lightbar ids.
    #[test]
    fn a18_lightbar_packet() {
        let colors = [
            Color(1, 1, 1),
            Color(2, 2, 2),
            Color(3, 3, 3),
            Color(4, 4, 4),
        ];
        let p = zone_packet(Kind::Alc, &A18_ALC_IDS, &colors);
        assert_eq!(p[..5], [0x00, 0x0C, 0x00, 0x06, 0x00]);
        assert_eq!(p[5..21], [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4]);
        assert_eq!(p[21], 0xFF);
    }

    // MSILaptopController::SetLEDs, KLC_LIGHTBAR branch: 53 LEDs as 30 + 23, then the commit.
    #[test]
    fn ge68hx_lightbar_packets() {
        let ids: Vec<u8> = (0..=0x34).collect();
        let colors: Vec<Color> = (0..53).map(|i| Color(i, 100, 200)).collect();
        let ops = lightbar_packets(&ids, &colors);
        assert_eq!(ops.len(), 3);
        let Op::Feature(a) = &ops[0] else { panic!() };
        assert_eq!(a.len(), 525);
        assert_eq!(a[..5], [0x00, 0x0E, 0x00, 30, 0x00]);
        assert_eq!(a[5..17], [0, 100, 200, 0, 0, 0, 0, 0, 0, 1, 0, 0]);
        assert_eq!(
            a[5 + 29 * 12..5 + 30 * 12],
            [29, 100, 200, 0, 0, 0, 0, 0, 0, 1, 0, 29]
        );
        assert!(a[5 + 30 * 12..].iter().all(|&b| b == 0));
        let Op::Feature(b) = &ops[1] else { panic!() };
        assert_eq!(b[3], 23);
        assert_eq!(b[5..17], [30, 100, 200, 0, 0, 0, 0, 0, 0, 1, 0, 30]);
        let mut commit = vec![0u8; 65];
        commit[1] = 0x0D;
        commit[3] = 0x02;
        assert_eq!(ops[2], Op::Write(commit));
    }

    #[test]
    fn keyboard_layout_covers_every_key() {
        let (leds, grid) = matrix(KLC_IDS.len(), 6, 23, &klc_map());
        assert_eq!(leds.len(), 102);
        // ISO backslash (LED 99) is not in the matrix and gets its own row
        assert_eq!(grid, [23, 7]);
    }

    // Known laptop ids plus the two GE66 lightbars.
    #[test]
    fn detectors() {
        assert_eq!(DETECTORS.len(), 9);
        assert!(
            DETECTORS
                .iter()
                .all(|d| d.vid == 0x1038 && d.interface == Some(0) && d.usage_page.is_none())
        );
    }

    // GE66 11UH stream start once, then one 0x0C frame per push.
    #[test]
    fn ge66_stream_frames() {
        let mut l = Laptop {
            kind: Kind::StreamLightbar,
            ids: (0..35).collect(),
            started: false,
        };
        let colors: Vec<Color> = (0..35).map(|i| Color(i, 1, 2)).collect();
        let ops = l.frame(&colors);
        assert_eq!(ops.len(), 4);
        let Op::Write(r) = &ops[0] else { panic!() };
        assert_eq!(r[..6], [0, 0x51, 0, 0xFF, 0xFF, 0x99]);
        let Op::Feature(f) = &ops[3] else { panic!() };
        assert_eq!(f[..5], [0, 0x0C, 0, 35, 0]);
        assert_eq!(f[5 + 34 * 4..9 + 34 * 4], [34, 34, 1, 2]);
        assert_eq!(l.frame(&colors).len(), 1);
    }

    // GE76 Raider 11UH: one 0E 00 1E 00 report of 30 zones, ~25 ms, then 0D 00 02.
    #[test]
    fn ge76_lightbar_waits_before_commit() {
        let mut l = Laptop {
            kind: Kind::AlcLightbar,
            ids: (0..30).collect(),
            started: false,
        };
        let colors: Vec<Color> = (0..30).map(|i| Color(0xFF, i, 0x10)).collect();
        let ops = l.frame(&colors);
        assert_eq!(ops.len(), 3);
        let Op::Feature(f) = &ops[0] else { panic!() };
        assert_eq!(f.len(), 525);
        assert_eq!(f[1..5], [0x0E, 0x00, 0x1E, 0x00]);
        assert_eq!(f[5..17], [0xFF, 0, 0x10, 0, 0, 0, 0, 0, 0, 1, 0, 0x00]);
        assert_eq!(
            f[5 + 29 * 12..5 + 30 * 12],
            [0xFF, 29, 0x10, 0, 0, 0, 0, 0, 0, 1, 0, 0x1D]
        );
        assert_eq!(ops[1], Op::Sleep(25));
        let Op::Write(c) = &ops[2] else { panic!() };
        assert_eq!(c.len(), 65);
        assert_eq!(c[1..5], [0x0D, 0x00, 0x02, 0x00]);
    }

    // Raider GE78 HX: zones 0x00-0x1D then 0x1E-0x36, a commit after each report.
    #[test]
    fn ge78hx_lightbar_commits_each_report() {
        let ids: Vec<u8> = (0..55).collect();
        let colors = vec![Color(1, 2, 3); 55];
        let ops = lightbar_ops(&ids, &colors, true);
        assert_eq!(ops.len(), 6);
        let Op::Feature(a) = &ops[0] else { panic!() };
        assert_eq!(a[1..5], [0x0E, 0x00, 0x1E, 0x00]);
        let Op::Feature(b) = &ops[3] else { panic!() };
        assert_eq!(b[1..5], [0x0E, 0x00, 0x19, 0x00]);
        assert_eq!(b[5..17], [1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 0, 0x1E]);
        assert_eq!(b[5 + 24 * 12 + 11], 0x36);
        assert!(b[5 + 25 * 12..].iter().all(|&x| x == 0));
        for i in [2, 5] {
            assert_eq!(ops[i], Op::Write(commit()));
        }
    }

    // Titan 18 HX ALC: five touchpad zones and the logo, as captured from the vendor app.
    #[test]
    fn titan18hx_alc_matches_capture() {
        let mut colors = vec![Color(0xFF, 0, 0); 5];
        colors.push(Color(0x09, 0x00, 0xFF));
        let p = zone_packet(Kind::AlcZeroFill, &[0, 1, 2, 3, 4, 5], &colors);
        let mut want = vec![0u8; 525];
        want[1..29].copy_from_slice(&[
            0x0C, 0x00, 0x06, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x01, 0xFF, 0x00, 0x00, 0x02, 0xFF,
            0x00, 0x00, 0x03, 0xFF, 0x00, 0x00, 0x04, 0xFF, 0x00, 0x00, 0x05, 0x09, 0x00, 0xFF,
        ]);
        assert_eq!(p, want);
    }
}
