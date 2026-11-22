// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "tecknet";
const BRIGHTNESS_HIGH: u8 = 0x03;

/// SendUpdate in direct mode: inverted colour, full brightness, no speed.
pub fn update(c: Color) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..7].copy_from_slice(&[
        0x02,
        0x04,
        255 - c.0,
        255 - c.1,
        255 - c.2,
        BRIGHTNESS_HIGH,
        0,
    ]);
    b
}

struct Tecknet;

impl Proto for Tecknet {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&update(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Tecknet)
}

pub static DETECTORS: &[Detector] = &[det("Tecknet M008", 0x04D9, 0xFC05, ANY, 0xFFA0, 0x01, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // TecknetController::SetColor -> SendUpdate with the direct mode row
    #[test]
    fn colour_report() {
        let [first, _, next] = pushes(Tecknet, 1, vec![], &[Color(0, 1, 255)], &[Color(255, 0, 0)]);
        assert_eq!(first, vec![Op::F(pad(&[2, 4, 255, 254, 0, 3, 0], 16))]);
        assert_eq!(next, vec![Op::F(pad(&[2, 4, 0, 255, 255, 3, 0], 16))]);
    }
}
