//! Ported from OpenRGB Controllers/AresonController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "areson";
const STATIC: u8 = 0x02;
/// Static's default brightness 10, from the 1..10 table.
const BRIGHTNESS: u8 = 0xFF;

/// SetMode in Static (OpenRGB's per-LED mode here); static has no speed value.
pub fn set_static(c: Color) -> [u8; 17] {
    let mut b = [0u8; 17];
    b[..12].copy_from_slice(&[
        0x08, 0x07, 0, 0, 0xA0, 0x07, STATIC, c.0, c.1, c.2, 0, BRIGHTNESS,
    ]);
    b[12] = b[6..12].iter().fold(0x55u8, |a, &v| a.wrapping_sub(v));
    b[16] = 0x4A;
    b
}

struct Areson;

impl Proto for Areson {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&set_static(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Areson)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "ZET GAMING Edge Air Pro (Wireless)",
        0x25A7,
        0xFA3F,
        1,
        0xFF02,
        2,
        open,
    ),
    det(
        "ZET GAMING Edge Air Pro",
        0x25A7,
        0xFA40,
        1,
        0xFF02,
        2,
        open,
    ),
    det(
        "ZET GAMING Edge Air Elit (Wireless)",
        0x25A7,
        0xFA48,
        1,
        0xFF02,
        2,
        open,
    ),
    det(
        "ZET GAMING Edge Air Elit",
        0x25A7,
        0xFA49,
        1,
        0xFF02,
        2,
        open,
    ),
    det(
        "Redragon M914 NIX (Wireless)",
        0x25A7,
        0xFA7C,
        1,
        0xFF02,
        2,
        open,
    ),
    det("Redragon M914 NIX", 0x25A7, 0xFA7B, 1, 0xFF02, 2, open),
    det("Redragon M901P-KS", 0x25A7, 0xFA08, 1, 0xFF02, 2, open),
    det(
        "Redragon M901P-KS (Wireless)",
        0x25A7,
        0xFA07,
        1,
        0xFF02,
        2,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // AresonController::SetMode(Static)
    #[test]
    fn static_report() {
        let [first, _, _] = pushes(Areson, 1, vec![], &[Color(1, 2, 3)], &[]);
        // 0x55 - 2 - 1 - 2 - 3 - 0 - 0xFF = 0x4E (mod 256)
        let e = vec![
            8, 7, 0, 0, 0xA0, 7, 2, 1, 2, 3, 0, 0xFF, 0x4E, 0, 0, 0, 0x4A,
        ];
        assert_eq!(first, vec![Op::F(e)]);
    }
}
