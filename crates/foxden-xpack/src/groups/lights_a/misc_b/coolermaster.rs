//! Cooler Master CK720, CK721, MK850, MP860 and the Acer P09-600 fan hub (protocol learned from SignalRGB's CoolerMaster_CK720_Keyboard.js, CoolerMaster_CK721_Keyboard.js, CoolerMaster_Mk850_Keyboard.js, CoolerMaster_MP860_Mat.js and ACER_P09-600_Controller.js).

use super::super::common::{KEYBOARD_RECT, MOUSE_RECT, SMALL_RECT, full, keyed};
use super::layouts::*;
use crate::hid::{Driver, Io, Proto, on_i, open_io};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x2516;
const LEN: usize = 65;
const PAGE: u16 = 0xFF00;
const APPLY: [u8; 6] = [0x00, 0x51, 0x28, 0x00, 0x00, 0xFF];

/// `stride` bytes per LED at `index * stride` in a buffer of `bytes`; a fourth byte is 0xFF.
fn indexed(bytes: usize, stride: usize, leds: &[u8], colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; bytes];
    for (i, c) in leds.iter().zip(colors) {
        let at = *i as usize * stride;
        if at + stride <= b.len() {
            b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
            if stride == 4 {
                b[at + 3] = 0xFF;
            }
        }
    }
    b
}

fn with(head: &[u8], data: &[u8]) -> Vec<u8> {
    let mut p = head.to_vec();
    p.extend_from_slice(data);
    p
}

/// CK720 host-control mode and custom profile, re-sent before every frame.
fn ck720_mode() -> [Vec<u8>; 2] {
    let profile = [
        0x00, 0x56, 0x81, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0xBB, 0xBB,
        0xBB, 0xBB,
    ];
    [vec![0x00, 0x41, 0x05], profile.to_vec()]
}

const CK720_STREAM: [u8; 20] = [
    0x01, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0xC1, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
    0x00, 0x00, 0x00, 0x00,
];

/// CK720: mode, `0x56 0x83 0` with the stream header and 40 bytes, nine `0x56 0x83 n` of 60, then apply.
pub fn ck720_frame(colors: &[Color]) -> Vec<Vec<u8>> {
    let b = indexed(40 + 9 * 60, 3, CK720, colors);
    let mut v = ck720_mode().to_vec();
    v.push(with(
        &[0x00, 0x56, 0x83, 0x00, 0x00],
        &[&CK720_STREAM[..], &b[..40]].concat(),
    ));
    for (n, ch) in b[40..].chunks(60).enumerate() {
        v.push(with(&[0x00, 0x56, 0x83, n as u8 + 1, 0x00], ch));
    }
    v.push(APPLY.to_vec());
    v
}

/// CK721: RGBA per LED, `0x0C 0x85` with 58 bytes then four `0x0C n` of 62.
pub fn ck721_frame(colors: &[Color]) -> Vec<Vec<u8>> {
    let b = indexed(58 + 4 * 62, 4, CK721, colors);
    let mut v = vec![with(&[0x00, 0x0C, 0x85, 0x00, 0x00, 0x1C, 0x01], &b[..58])];
    for (n, ch) in b[58..].chunks(62).enumerate() {
        v.push(with(&[0x00, 0x0C, n as u8 + 1], ch));
    }
    v
}

/// MK850: eight `0x56 0x83 n` packets of 60 bytes (each answered), then apply.
pub fn mk850_frame(colors: &[Color]) -> Vec<Vec<u8>> {
    let b = indexed(8 * 60, 3, MK850, colors);
    b.chunks(60)
        .enumerate()
        .map(|(n, ch)| with(&[0x00, 0x56, 0x83, n as u8 + 1, 0x00], ch))
        .collect()
}

/// MP860: custom mode, `0x51 0xA8` packets 0, 2, 4 of 12, 12 and 6 LEDs, then apply `0xB0`.
pub fn mp860_frame(colors: &[Color]) -> Vec<Vec<u8>> {
    let b = indexed(30 * 3, 3, MP860, colors);
    let mut v = vec![vec![0x00, 0x41, 0x80]];
    for (n, ch) in b.chunks(36).enumerate() {
        v.push(with(&[0x00, 0x51, 0xA8, (n * 2) as u8, 0x00], ch));
    }
    v.push(vec![0x00, 0x51, 0x28, 0x00, 0x00, 0xB0]);
    v
}

/// P09-600: three single-colour 12 V fan headers in a fixed `0x56` sequence.
pub fn acer_frame(c: &[Color]) -> Vec<Vec<u8>> {
    let mut fan01 = vec![
        0x00, 0x56, 0x21, 0x00, 0x00, 0x00, 0x31, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0xFF, 0xFF,
        0xFF, 0xFF, 0x08, 0x00, 0x02, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xFF,
        0x00, 0x00, 0x00,
    ];
    let slot = [
        0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00,
    ];
    for col in &c[..2] {
        fan01.extend([col.0, col.1, col.2]);
        fan01.extend(slot);
    }
    let mut fan2 = vec![0x00, 0x56, 0x21, 0x01, 0x00, c[2].0, c[2].1, c[2].2];
    fan2.extend([
        0xFF, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00,
        0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x11, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05,
        0x00, 0x01, 0x00, 0x09, 0x00, 0x02, 0x00, 0x0D, 0x00, 0x03,
    ]);
    vec![
        vec![0x00, 0x41, 0x01],
        vec![0x00, 0x56, 0x14],
        vec![0x00, 0x56, 0x20],
        vec![0x00, 0x56, 0x20, 0x01],
        vec![0x00, 0x56, 0x20, 0x02],
        vec![0x00, 0x56, 0x15, 0x00, 0x00, 0x04, 0x00, 0x15],
        fan01,
        fan2,
        vec![0x00, 0x56, 0x21, 0x02],
        vec![0x00, 0x51, 0x28],
        vec![0x00, 0x50, 0x55],
    ]
}

#[derive(Clone, Copy, PartialEq)]
enum Model {
    Ck720,
    Ck721,
    Mk850,
    Mp860,
    Acer,
}

struct Cm(Model);

impl Proto for Cm {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        match self.0 {
            Model::Ck720 => {
                for p in ck720_mode() {
                    io.write(&p, LEN)?;
                }
            }
            Model::Ck721 => {
                io.write(&[0x00, 0x80, 0x01, 0x01, 0x02, 0x02], LEN)?;
                io.write(&[0x00, 0x01, 0x81, 0xC8, 0x00, 0x01, 0x00, 0x12], LEN)?;
            }
            Model::Mk850 => {
                io.write(&[0x00, 0x51, 0x00, 0x00, 0x00, 0x05], LEN)?;
                io.write(&APPLY, LEN)?;
                io.sleep(3);
            }
            _ => {}
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let packets = match self.0 {
            Model::Ck720 => ck720_frame(colors),
            Model::Ck721 => ck721_frame(colors),
            Model::Mk850 => mk850_frame(colors),
            Model::Mp860 => mp860_frame(colors),
            Model::Acer => acer_frame(colors),
        };
        for p in packets {
            io.write(&p, LEN)?;
            if self.0 == Model::Mk850 {
                let _ = io.read(LEN, 50);
            }
        }
        if self.0 == Model::Mk850 {
            io.write(&APPLY, LEN)?;
            io.sleep(3);
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let restore: &[&[u8]] = match self.0 {
            Model::Mk850 => &[&[0x00, 0x41, 0x80], &[0x00, 0x51, 0x28, 0x00, 0x00, 0x01]],
            Model::Acer => &[&[0x00, 0x50, 0x55], &[0x00, 0x41]],
            _ => &[],
        };
        for p in restore {
            let _ = io.write(p, LEN);
        }
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (model, kind, rect, size, cells): (Model, DeviceKind, [f32; 4], [u32; 2], &[[u8; 2]]) =
        match info.pid {
            0x016B => (
                Model::Ck720,
                DeviceKind::Keyboard,
                KEYBOARD_RECT,
                [15, 5],
                CK720_CELLS,
            ),
            0x016D => (
                Model::Ck721,
                DeviceKind::Keyboard,
                KEYBOARD_RECT,
                [15, 5],
                CK721_CELLS,
            ),
            0x0069 => (
                Model::Mk850,
                DeviceKind::Keyboard,
                KEYBOARD_RECT,
                [21, 6],
                MK850_CELLS,
            ),
            0x006D => (
                Model::Mp860,
                DeviceKind::Other,
                MOUSE_RECT,
                [6, 6],
                MP860_CELLS,
            ),
            _ => (
                Model::Acer,
                DeviceKind::Other,
                SMALL_RECT,
                [3, 1],
                &[[0, 0], [1, 0], [2, 0]],
            ),
        };
    let dev = keyed(
        &info.device_id("coolermaster"),
        d.name,
        kind,
        rect,
        size,
        cells,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "coolermaster", Cm(model)).boxed())
}

pub static DETECTORS: &[Detector] = &[
    full(
        "ACER P09-600 RGB controller",
        VID,
        0x010B,
        1,
        PAGE,
        0x0001,
        open,
    ),
    full("CoolerMaster CK720", VID, 0x016B, 1, PAGE, 0x0001, open),
    full("CoolerMaster CK721", VID, 0x016D, 3, PAGE, 0x0002, open),
    on_i("CoolerMaster MK850", VID, 0x0069, 1, open),
    on_i("Cooler Master MP860 Mouse Pad", VID, 0x006D, 1, open),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ck720_layout() {
        let f = ck720_frame(&[Color(1, 2, 3)]);
        assert_eq!(f.len(), 2 + 1 + 9 + 1);
        assert_eq!(f[2][..7], [0, 0x56, 0x83, 0, 0, 0x01, 0x00]);
        assert_eq!(f[2][25..28], [1, 2, 3]);
        assert_eq!(f[11][..4], [0, 0x56, 0x83, 9]);
        assert_eq!(f[12], APPLY.to_vec());
    }

    #[test]
    fn ck721_rgba_and_mp860() {
        let f = ck721_frame(&[Color(1, 2, 3)]);
        assert_eq!(f.len(), 5);
        assert_eq!(f[0][7..11], [1, 2, 3, 0xFF]);
        let m = mp860_frame(&[Color(4, 5, 6); 19]);
        assert_eq!(
            m.iter().map(|p| p.get(3).copied()).collect::<Vec<_>>(),
            vec![None, Some(0), Some(2), Some(4), Some(0)]
        );
        assert_eq!(m[3].len(), 5 + 18);
    }

    #[test]
    fn acer_sequence() {
        let f = acer_frame(&[Color(1, 1, 1), Color(2, 2, 2), Color(3, 3, 3)]);
        assert_eq!(f.len(), 11);
        assert_eq!(f[6][33..36], [1, 1, 1]);
        assert_eq!(f[6][49..52], [2, 2, 2]);
        assert_eq!(f[7][5..8], [3, 3, 3]);
        assert_eq!(f[6].len(), 65);
    }
}
