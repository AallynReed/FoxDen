//! Ported from OpenRGB Controllers/RedSquareKeyroxController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "keyrox";
const LEN: usize = 520;
const CUSTOM: u8 = 0x0C;
/// Custom stores brightness per LED, at its 0xFF maximum.
const BRIGHTNESS: u8 = 0xFF;

const MAP: [&[u32]; 6] = [
    &[0, NA, 1, 2, 3, 4, 5, 6, 7, 8, NA, 9, 10, 11, 12, 13, 14, 15],
    &[
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, NA, 30, 31, 32,
    ],
    &[
        33, NA, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    ],
    &[
        50, NA, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, NA, 62, NA, NA, NA,
    ],
    &[
        63, NA, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, NA, 74, NA, NA, 75, NA,
    ],
    &[
        76, 77, 78, NA, NA, NA, 79, NA, NA, NA, 80, 81, NA, 82, 83, 84, 85, 86,
    ],
];

const POSITIONS: [usize; 87] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 70, 51, 52, 53,
    58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 50, 75, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86,
    88, 89, 94, 95, 96, 98, 100, 101, 102, 103, 104, 105, 106,
];

/// Send: report 0 then the 520-byte payload.
fn report(data: &[u8; LEN]) -> Vec<u8> {
    let mut b = vec![0u8];
    b.extend_from_slice(data);
    b
}

pub fn set_mode(mode: u8) -> Vec<u8> {
    let mut d = [0u8; LEN];
    d[4] = 0x01;
    d[6] = 0x04;
    d[8] = mode;
    report(&d)
}

pub fn leds_data(colors: &[Color]) -> Vec<u8> {
    let mut d = [0u8; LEN];
    d[4] = 0xB0;
    d[5] = 0x01;
    d[6] = 0x07;
    for (&p, c) in POSITIONS.iter().zip(colors) {
        let o = 7 + p * 4;
        d[o + 1..o + 5].copy_from_slice(&[c.0, c.1, c.2, BRIGHTNESS]);
    }
    report(&d)
}

struct Keyrox;

impl Proto for Keyrox {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&set_mode(CUSTOM))?;
        io.sleep(10);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&leds_data(colors))?;
        io.sleep(10);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MAP, POSITIONS.len());
    boxed(api, info, FAMILY, dev, Keyrox)
}

pub static DETECTORS: &[Detector] = &[
    det("Red Square Keyrox TKL", 0x1A2C, 0x1511, 3, 0xFF00, 2, open),
    det(
        "Red Square Keyrox TKL V2",
        0x1A2C,
        0x2511,
        3,
        0xFF00,
        2,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DeviceUpdateMode(Custom) -> SetMode, then SetLEDsData
    #[test]
    fn mode_then_leds() {
        let mut a = vec![Color::BLACK; 87];
        a[46] = Color(1, 2, 3); // position 70
        let [first, _, _] = pushes(Keyrox, 87, vec![], &a, &[]);
        assert_eq!(
            first[0],
            Op::F(pad(&[0, 0, 0, 0, 0, 1, 0, 4, 0, 0x0C], 521))
        );
        assert_eq!(first[1], Op::Sleep(10));
        let Op::F(b) = &first[2] else { panic!() };
        assert_eq!(b.len(), 521);
        assert_eq!(b[5..8], [0xB0, 1, 7]);
        let o = 1 + 7 + 70 * 4;
        assert_eq!(b[o + 1..o + 5], [1, 2, 3, 0xFF]);
        assert_eq!(b[1 + 7 + 1..1 + 7 + 5], [0, 0, 0, 0xFF]);
        assert_eq!(first[3], Op::Sleep(10));
    }
}
