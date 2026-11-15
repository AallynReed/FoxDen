//! Ported from OpenRGB Controllers/InstantMouseController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "instant-mouse";
const REPORT: u8 = 0x07;
const DIRECT: u8 = 0x0A;
const BRIGHTNESS_MAX: u8 = 0x07;

pub fn set_mode(mode: u8, speed: u8, brightness: u8, direction: u8) -> [u8; 8] {
    [
        REPORT,
        0x13,
        0xFF,
        (mode << 4) | (speed & 0xF),
        (direction << 4) | 0xB,
        0xF - (brightness & 0xF),
        mode & 0xF0,
        0x00,
    ]
}

/// SendColor: inverted 4-bit channels, repeated for slots 0, 2, ... 10.
pub fn send_color(c: Color) -> Vec<[u8; 8]> {
    let (r, g, b) = (0xF - c.0 / 16, 0xF - c.1 / 16, 0xF - c.2 / 16);
    (0..=0xAu8)
        .step_by(2)
        .map(|i| [REPORT, 0x14, (i << 4) | g, (r << 4) | b, 0, 0, 0, 0])
        .collect()
}

struct InstantMouse;

impl Proto for InstantMouse {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&set_mode(DIRECT, 0, BRIGHTNESS_MAX, 0))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send_color(colors[0]).iter().try_for_each(|p| io.feature(p))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, InstantMouse)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "Advanced GTA 250 USB Gaming Mouse",
        0x30FA,
        0x1030,
        1,
        0xFF01,
        0x01,
        open,
    ),
    det(
        "Anko KM43243952 USB Gaming Mouse",
        0x30FA,
        0x1440,
        1,
        0xFF01,
        0x01,
        open,
    ),
    det(
        "Anko KM43277483 USB Gaming Mouse",
        0x30FA,
        0x1540,
        1,
        0xFF01,
        0x01,
        open,
    ),
    det(
        "AntEsports GM600 USB Gaming Mouse",
        0x30FA,
        0x1040,
        1,
        0xFF01,
        0x01,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DeviceUpdateMode(Direct) -> SetMode, then SendColor
    #[test]
    fn mode_then_colour() {
        let [first, _, _] = pushes(InstantMouse, 1, vec![], &[Color(255, 16, 0)], &[]);
        assert_eq!(
            first[0],
            Op::F(vec![7, 0x13, 0xFF, 0xA0, 0x0B, 0x08, 0x00, 0])
        );
        // red 0xF - 15 = 0, green 0xF - 1 = 0xE, blue 0xF
        assert_eq!(first[1], Op::F(vec![7, 0x14, 0x0E, 0x0F, 0, 0, 0, 0]));
        assert_eq!(first[6], Op::F(vec![7, 0x14, 0xAE, 0x0F, 0, 0, 0, 0]));
        assert_eq!(first.len(), 7);
    }
}
