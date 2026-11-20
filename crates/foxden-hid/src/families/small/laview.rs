//! Ported from OpenRGB Controllers/LaviewTechnologyController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "laview";
const STATIC: u8 = 0x01;
/// Custom mode's default brightness; its speed is left at 0.
const BRIGHTNESS: u8 = 50;

pub fn set_mode(mode: u8, brightness: u8, speed: u8, c: Color) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0xA1;
    b[1] = 0x0C;
    b[5] = 0x01;
    b[16] = mode;
    b[17..20].copy_from_slice(&rgb(c));
    b[56] = brightness;
    b[58] = speed;
    b
}

struct Laview;

impl Proto for Laview {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&set_mode(STATIC, BRIGHTNESS, 0, colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Laview)
}

pub static DETECTORS: &[Detector] = &[det(
    "Glorious Model I",
    0x22D4,
    0x1503,
    1,
    0xFF01,
    0x02,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // LaviewTechnologyController::SetMode for the Custom mode
    #[test]
    fn custom_mode_report() {
        let [first, _, _] = pushes(Laview, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        let mut e = pad(&[0xA1, 0x0C, 0, 0, 0, 0x01], 64);
        e[16] = 0x01;
        e[17..20].copy_from_slice(&[1, 2, 3]);
        e[56] = 50;
        assert_eq!(first, vec![Op::F(e)]);
    }
}
