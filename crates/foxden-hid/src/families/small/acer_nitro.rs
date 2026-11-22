// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "acer-nitro";
const KEYBOARD: u8 = 0x21;
const LED: u8 = 0x65;
const STATIC: u8 = 2;
const BRIGHTNESS: u8 = 100;

/// SetLEDs: one static-mode report per zone, selected by bit.
pub fn zone_report(device: u8, zone: usize, c: Color) -> [u8; 11] {
    [
        0xA4,
        device,
        STATIC,
        BRIGHTNESS,
        0,
        1,
        c.0,
        c.1,
        c.2,
        1 << zone,
        0,
    ]
}

/// The keyboard's four zones then the lid LED, which OpenRGB lists as a second device.
struct AcerNitro;

impl Proto for AcerNitro {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (z, &c) in colors[..4].iter().enumerate() {
            io.feature(&zone_report(KEYBOARD, z, c))?;
        }
        io.feature(&zone_report(LED, 0, colors[4]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Keyboard,
        4,
        2,
        &[(0, 1), (1, 1), (2, 1), (3, 1), (1, 0)],
    );
    boxed(api, info, FAMILY, dev, AcerNitro)
}

pub static DETECTORS: &[Detector] = &[det(
    "AcerNitroHidKeyboardController",
    0x0CF2,
    0x5130,
    ANY,
    0xFF5A,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // AcerNitroHidKeyboardController::SetLEDs for device 0x21 (4 zones) and 0x65 (1 LED)
    #[test]
    fn five_reports() {
        let a: Vec<Color> = (0..5).map(|i| Color(i, i + 1, i + 2)).collect();
        let [first, _, _] = pushes(AcerNitro, 5, vec![], &a, &[]);
        assert_eq!(
            first,
            vec![
                Op::F(vec![0xA4, 0x21, 2, 100, 0, 1, 0, 1, 2, 1, 0]),
                Op::F(vec![0xA4, 0x21, 2, 100, 0, 1, 1, 2, 3, 2, 0]),
                Op::F(vec![0xA4, 0x21, 2, 100, 0, 1, 2, 3, 4, 4, 0]),
                Op::F(vec![0xA4, 0x21, 2, 100, 0, 1, 3, 4, 5, 8, 0]),
                Op::F(vec![0xA4, 0x65, 2, 100, 0, 1, 4, 5, 6, 1, 0]),
            ]
        );
    }
}
