// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Siberia,
    Arctis5,
    ArctisNova3,
    QckMat,
    QckPrism12,
}

const fn det(name: &'static str, pid: u16, interface: i32) -> Detector {
    Detector {
        name,
        vid: 0x1038,
        pid,
        interface: Some(interface),
        usage_page: None,
        usage: None,
        open,
    }
}

static DETECTORS: [Detector; 15] = [
    det("SteelSeries Siberia 350", 0x1229, 3),
    Detector {
        usage_page: Some(0xFFC0),
        usage: Some(1),
        ..det("SteelSeries Arctis Nova 3", 0x12EC, 4)
    },
    det("SteelSeries Arctis 5", 0x1250, 5),
    det("SteelSeries Arctis 5", 0x12AA, 5),
    det("SteelSeries QCK Prism Cloth Medium", 0x150A, 0),
    det("SteelSeries QCK Prism Cloth XL", 0x150D, 0),
    det("SteelSeries QCK Prism Cloth XL Destiny Ed.", 0x151E, 0),
    det(
        "SteelSeries QCK Prism Cloth XL Destiny 2 Lightfall Ed.",
        0x1520,
        0,
    ),
    det(
        "SteelSeries QCK Prism Cloth XL CS:GO Neon Rider Ed.",
        0x1514,
        0,
    ),
    det(
        "SteelSeries QCK Prism Cloth XL CS:GO Neo Noir Ed.",
        0x151C,
        0,
    ),
    det("SteelSeries QCK Prism Cloth 3XL", 0x1516, 0),
    det("SteelSeries QCK Prism Cloth 4XL", 0x1518, 0),
    det("SteelSeries QCK Prism Cloth 5XL", 0x151A, 0),
    det("SteelSeries QCK Prism Cloth XL", 0x3769, 0),
    det("SteelSeries QCK Prism", 0x1507, 0),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

fn kind(pid: u16) -> Option<Kind> {
    Some(match pid {
        0x1229 => Kind::Siberia,
        0x12EC => Kind::ArctisNova3,
        0x1250 | 0x12AA => Kind::Arctis5,
        0x150A | 0x150D | 0x151E | 0x1520 | 0x1514 | 0x151C | 0x1516 | 0x1518 | 0x151A | 0x3769 => {
            Kind::QckMat
        }
        0x1507 => Kind::QckPrism12,
        _ => return None,
    })
}

fn padded(len: usize, head: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[..head.len()].copy_from_slice(head);
    v
}

/// SteelSeriesSiberiaController's send_usb_msg: [0x01, 0x00, command, length, data...] in 16 bytes.
fn siberia_msg(data: &[u8]) -> Op {
    let mut p = vec![0x01, 0x00, data[0], (data.len() - 1) as u8];
    p.extend_from_slice(&data[1..]);
    p.resize(16, 0);
    Op::Write(p)
}

/// SteelSeriesSiberiaController::SetColor.
fn siberia(c: Color) -> Vec<Op> {
    vec![
        siberia_msg(&[0x95, 0x80, 0xBF]),
        siberia_msg(&[0x80, 0x52, 0x20]),
        siberia_msg(&[0x83, c.0, c.1, c.2]),
        siberia_msg(&[0x93, 0x03, 0x80]),
    ]
}

/// SteelSeriesArctis5Controller::SetColor.
fn arctis5(zone: u8, c: Color) -> Vec<Op> {
    let w = |head: &[u8]| Op::Write(padded(37, head));
    vec![
        w(&[0x06, 0x81, 0x43, 0x01, 0x22]),
        w(&[0x06, 0x81, 0x43, 0x01, 0x23]),
        w(&[
            0x06, 0x8A, 0x42, 0x00, 0x20, 0x41, 0x00, c.0, c.1, c.2, 0xFF, 0x32, 0xC8, 0xC8,
        ]),
        w(&[0x06, 0x8A, 0x42, 0x00, 0x20, 0x41, 0x08, zone, 0x01]),
        w(&[0x06, 0x8A, 0x42, 0x00, 0x20, 0x60, zone]),
        w(&[0x06, 0x8A, 0x42, 0x00, 0x20, 0x05]),
    ]
}

/// SteelSeriesArctisNova3Controller::SetDirect: effect packet, SelectZone, ApplyZone.
fn nova3(zone: u8, c: Color) -> Vec<Op> {
    let mut e = padded(521, &[0x06, 0xAA, if zone == 0 { 0x01 } else { 0x00 }]);
    e[0x0D] = 0x98;
    e[0x0E] = 0x3A;
    e[0x15] = 0x55;
    e[0x17] = 0x96;
    e[0x1D] = 0x01;
    e[0x25] = 0x02;
    e[0x2D..0x30].copy_from_slice(&[c.0, c.1, c.2]);
    e[0x30] = 0xFF;
    vec![
        Op::Write(e),
        Op::Write(padded(
            64,
            &[0x06, 0xA5, if zone == 0 { 0x02 } else { 0x01 }, 0x0A],
        )),
        Op::Write(padded(64, &[0x06, if zone == 0 { 0x09 } else { 0xA3 }])),
    ]
}

/// SteelSeriesQCKMatController::SetColors: one feature report, then the commit.
fn qck(bottom: Color, top: Color) -> Vec<Op> {
    let mut b = vec![0u8; 525];
    b[0x01] = 0x0E;
    b[0x03] = 0x02;
    b[0x05..0x08].copy_from_slice(&[bottom.0, bottom.1, bottom.2]);
    b[0x08] = 0xFF;
    b[0x09] = 0x32;
    b[0x0A] = 0xC8;
    b[0x0E] = 0x01;
    b[0x11..0x14].copy_from_slice(&[top.0, top.1, top.2]);
    b[0x14] = 0xFF;
    b[0x15] = 0x32;
    b[0x16] = 0xC8;
    b[0x19] = 0x01;
    b[0x1A] = 0x01;
    b[0x1C] = 0x01;
    vec![Op::Feature(b), Op::Write(padded(65, &[0x00, 0x0D]))]
}

/// 12-LED QcK Prism ring (Steelseries_Qck_Prism_Mousepad.js sendColors): 12-byte entries, then the commit.
fn qck_prism12(colors: &[Color]) -> Vec<Op> {
    let mut b = vec![0u8; 525];
    b[0x01] = 0x0E;
    b[0x03] = PRISM12_RING.len() as u8;
    for i in 0..PRISM12_RING.len() {
        let c = colors.get(i).copied().unwrap_or(Color::BLACK);
        let at = 5 + i * 12;
        b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
        b[at + 9] = 0x01;
        b[at + 11] = i as u8;
    }
    vec![Op::Feature(b), Op::Write(padded(65, &[0x00, 0x0D]))]
}

/// LED positions on the plugin's 5x5 canvas, starting at the left edge and running anticlockwise.
const PRISM12_RING: [(u8, u8); 12] = [
    (0, 2),
    (0, 3),
    (1, 4),
    (2, 4),
    (3, 4),
    (4, 3),
    (4, 2),
    (4, 1),
    (3, 1),
    (2, 1),
    (1, 1),
    (0, 1),
];

fn prism12_layout() -> (Vec<[f32; 2]>, [u32; 2]) {
    let leds = PRISM12_RING
        .iter()
        .map(|&(x, y)| [(x as f32 + 0.5) / 5.0, (y as f32 + 0.5) / 5.0])
        .collect();
    (leds, [5, 5])
}

struct Accessory {
    kind: Kind,
}

impl Proto for Accessory {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let c = |i: usize| colors.get(i).copied().unwrap_or(Color::BLACK);
        match self.kind {
            Kind::Siberia => siberia(c(0)),
            Kind::Arctis5 => [arctis5(0, c(0)), arctis5(1, c(1))].concat(),
            Kind::ArctisNova3 => [nova3(0, c(0)), nova3(1, c(1))].concat(),
            Kind::QckMat => qck(c(0), c(1)),
            Kind::QckPrism12 => qck_prism12(colors),
        }
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let kind = kind(info.pid).ok_or("unknown SteelSeries device")?;
    let hid = info.open(api)?;
    let layout = match kind {
        Kind::Siberia => line(1),
        Kind::QckPrism12 => prism12_layout(),
        _ => line(2),
    };
    let dev = device(
        info.device_id("steelseries"),
        det.name,
        DeviceKind::Other,
        layout,
    );
    Ok(Driver::boxed(hid, "steelseries", dev, Accessory { kind }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_detector_has_a_kind() {
        assert!(DETECTORS.iter().all(|d| kind(d.pid).is_some()));
        let nova = &DETECTORS[1];
        assert_eq!(
            (nova.interface, nova.usage_page, nova.usage),
            (Some(4), Some(0xFFC0), Some(1))
        );
    }

    // SteelSeriesSiberiaController::SetColor.
    #[test]
    fn siberia_bytes() {
        let ops = siberia(Color(1, 2, 3));
        let row = |head: &[u8]| Op::Write(padded(16, head));
        assert_eq!(
            ops,
            vec![
                row(&[0x01, 0x00, 0x95, 0x02, 0x80, 0xBF]),
                row(&[0x01, 0x00, 0x80, 0x02, 0x52, 0x20]),
                row(&[0x01, 0x00, 0x83, 0x03, 1, 2, 3]),
                row(&[0x01, 0x00, 0x93, 0x02, 0x03, 0x80]),
            ]
        );
    }

    // SteelSeriesArctis5Controller::SetColor for the right side.
    #[test]
    fn arctis5_bytes() {
        let ops = arctis5(1, Color(10, 20, 30));
        assert_eq!(ops.len(), 6);
        assert_eq!(
            ops[2],
            Op::Write(padded(
                37,
                &[
                    6, 0x8A, 0x42, 0, 0x20, 0x41, 0, 10, 20, 30, 0xFF, 0x32, 0xC8, 0xC8
                ]
            ))
        );
        assert_eq!(
            ops[3],
            Op::Write(padded(37, &[6, 0x8A, 0x42, 0, 0x20, 0x41, 0x08, 1, 0x01]))
        );
        assert_eq!(
            ops[4],
            Op::Write(padded(37, &[6, 0x8A, 0x42, 0, 0x20, 0x60, 1]))
        );
        assert_eq!(
            ops[5],
            Op::Write(padded(37, &[6, 0x8A, 0x42, 0, 0x20, 0x05]))
        );
    }

    // SteelSeriesArctisNova3Controller::SetDirect for both zones.
    #[test]
    fn nova3_bytes() {
        let ops = nova3(0, Color(7, 8, 9));
        let Op::Write(e) = &ops[0] else { panic!() };
        assert_eq!(e.len(), 521);
        assert_eq!(e[..3], [6, 0xAA, 0x01]);
        assert_eq!(e[0x0D..0x0F], [0x98, 0x3A]);
        assert_eq!(
            (e[0x15], e[0x17], e[0x1D], e[0x25]),
            (0x55, 0x96, 0x01, 0x02)
        );
        assert_eq!(e[0x2D..0x31], [7, 8, 9, 0xFF]);
        assert_eq!(e.iter().filter(|&&b| b != 0).count(), 13);
        assert_eq!(ops[1], Op::Write(padded(64, &[6, 0xA5, 0x02, 0x0A])));
        assert_eq!(ops[2], Op::Write(padded(64, &[6, 0x09])));
        let right = nova3(1, Color(7, 8, 9));
        let Op::Write(e) = &right[0] else { panic!() };
        assert_eq!(e[2], 0x00);
        assert_eq!(right[1], Op::Write(padded(64, &[6, 0xA5, 0x01, 0x0A])));
        assert_eq!(right[2], Op::Write(padded(64, &[6, 0xA3])));
    }

    // SteelSeriesQCKMatController::SetColors.
    #[test]
    fn qck_bytes() {
        let ops = qck(Color(1, 2, 3), Color(4, 5, 6));
        let Op::Feature(b) = &ops[0] else { panic!() };
        assert_eq!(b.len(), 525);
        assert_eq!(
            b[..0x1D],
            [
                0, 0x0E, 0, 0x02, 0, 1, 2, 3, 0xFF, 0x32, 0xC8, 0, 0, 0, 0x01, 0, 0, 4, 5, 6, 0xFF,
                0x32, 0xC8, 0, 0, 0x01, 0x01, 0, 0x01
            ]
        );
        assert!(b[0x1D..].iter().all(|&x| x == 0));
        assert_eq!(ops[1], Op::Write(padded(65, &[0, 0x0D])));
    }

    // Steelseries_Qck_Prism_Mousepad.js sendColors.
    #[test]
    fn qck_prism12_bytes() {
        let colors: Vec<Color> = (0..12).map(|i| Color(i + 1, 2, 3)).collect();
        let ops = qck_prism12(&colors);
        let Op::Feature(b) = &ops[0] else { panic!() };
        assert_eq!(b.len(), 525);
        assert_eq!(b[..5], [0, 0x0E, 0, 12, 0]);
        assert_eq!(b[5..17], [1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 0, 0]);
        assert_eq!(
            b[5 + 11 * 12..5 + 12 * 12],
            [12, 2, 3, 0, 0, 0, 0, 0, 0, 1, 0, 11]
        );
        assert!(b[5 + 12 * 12..].iter().all(|&x| x == 0));
        assert_eq!(ops[1], Op::Write(padded(65, &[0, 0x0D])));
        assert_eq!(prism12_layout().0.len(), 12);
    }
}
