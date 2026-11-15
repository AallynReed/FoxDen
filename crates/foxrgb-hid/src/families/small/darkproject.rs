//! Ported from OpenRGB Controllers/DarkProject (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "darkproject";
const KEYS: usize = 87;
const RED_BLUE: usize = 5;
const GREEN: usize = 107;

const PACKET_MAP: [u8; KEYS] = [
    5, 11, 17, 23, 29, 35, 41, 47, 53, 59, 65, 71, 77, 83, 89, 95, 0, 6, 12, 18, 24, 30, 36, 42,
    48, 54, 60, 66, 72, 78, 84, 90, 96, 1, 7, 13, 19, 25, 31, 37, 43, 49, 55, 61, 67, 73, 79, 85,
    91, 97, 2, 8, 14, 20, 26, 32, 38, 44, 50, 56, 62, 68, 80, 3, 15, 21, 27, 33, 39, 45, 51, 57,
    63, 69, 81, 93, 4, 10, 16, 34, 52, 58, 64, 76, 88, 94, 100,
];

const MATRIX: [[u32; 18]; 6] = [
    [0, NA, 1, 2, 3, 4, 5, 6, 7, 8, NA, 9, 10, 11, 12, 13, 14, 15],
    [
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, NA, 30, 31, 32,
    ],
    [
        33, NA, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    ],
    [
        50, NA, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, NA, 62, NA, NA, NA,
    ],
    [
        63, NA, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, NA, 74, NA, NA, 75, NA,
    ],
    [
        76, 77, 78, NA, NA, NA, 79, NA, NA, NA, 80, 81, NA, 82, 83, 84, 85, 86,
    ],
];

/// Red and green go in one packet, blue in the other.
pub fn leds_direct(colors: &[Color]) -> [[u8; 256]; 2] {
    let mut rg = [0u8; 256];
    let mut ba = [0u8; 256];
    rg[..5].copy_from_slice(&[0x08, 0x07, 0x00, 0x00, 0x00]);
    ba[..5].copy_from_slice(&[0x08, 0x07, 0x00, 0x01, 0x00]);
    for (c, &o) in colors.iter().zip(PACKET_MAP.iter()) {
        let o = o as usize;
        rg[RED_BLUE + o] = c.0;
        rg[GREEN + o] = c.1;
        ba[RED_BLUE + o] = c.2;
    }
    [rg, ba]
}

struct DarkProject;

impl Proto for DarkProject {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        leds_direct(colors).iter().try_for_each(|p| io.write(p))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let map: Vec<&[u32]> = MATRIX.iter().map(|r| &r[..]).collect();
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &map, KEYS);
    boxed(api, info, FAMILY, dev, DarkProject)
}

pub static DETECTORS: &[Detector] = &[det(
    "Dark Project KD3B V2",
    0x195D,
    0x2061,
    2,
    0xFFC2,
    4,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DarkProjectKeyboardController::SetLedsDirect
    #[test]
    fn two_packets() {
        let mut a = vec![Color::BLACK; KEYS];
        a[0] = Color(1, 2, 3);
        a[86] = Color(4, 5, 6);
        let [first, _, _] = pushes(DarkProject, KEYS, vec![], &a, &[]);
        let mut rg = pad(&[8, 7, 0, 0, 0], 256);
        let mut ba = pad(&[8, 7, 0, 1, 0], 256);
        rg[10] = 1;
        rg[112] = 2;
        ba[10] = 3;
        rg[105] = 4;
        rg[207] = 5;
        ba[105] = 6;
        assert_eq!(first, vec![w(rg), w(ba)]);
    }
}
