//! Ported from OpenRGB Controllers/ClevoLightbarController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "clevo-lightbar";
const DIRECT: u8 = 0x01;
const SPEED_MIN: u8 = 1;
const SPEED_MAX: u8 = 10;

pub fn set_color(c: Color) -> [u8; 8] {
    [0x14, 0x00, 0x01, c.0, c.1, c.2, 0, 0]
}

pub fn set_mode(mode: u8, brightness: u8, speed: u8) -> [u8; 8] {
    [
        0x08,
        0x22,
        mode,
        SPEED_MAX + SPEED_MIN - speed,
        brightness,
        0x01,
        0,
        0,
    ]
}

struct Lightbar;

impl Proto for Lightbar {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&set_color(colors[0]))?;
        // Direct keeps speed 0 and brightness 100
        io.feature(&set_mode(DIRECT, 100, 0))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Strip, 1);
    boxed(api, info, FAMILY, dev, Lightbar)
}

pub static DETECTORS: &[Detector] = &[det(
    "CLEVO Lightbar",
    0x048D,
    0x7001,
    ANY,
    0xFF03,
    0x02,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_ClevoLightbar::DeviceUpdateLEDs: SetColor then SetMode
    #[test]
    fn colour_then_mode() {
        let [first, _, _] = pushes(Lightbar, 1, vec![], &[Color(1, 2, 3)], &[]);
        assert_eq!(
            first,
            vec![
                Op::F(vec![0x14, 0, 1, 1, 2, 3, 0, 0]),
                Op::F(vec![0x08, 0x22, 1, 11, 100, 1, 0, 0])
            ]
        );
    }
}
