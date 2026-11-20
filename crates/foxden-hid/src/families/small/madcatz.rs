//! Ported from OpenRGB Controllers/MadCatzCyborgController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "madcatz-cyborg";
const ENABLE: [u8; 2] = [0xA1, 0x00];

pub fn intensity(v: u8) -> [u8; 3] {
    [0xA6, 0x00, v.min(100)]
}

pub fn led_color(c: Color) -> [u8; 9] {
    [0xA2, 0x00, c.0, c.1, c.2, 0, 0, 0, 0]
}

struct Cyborg;

impl Proto for Cyborg {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&ENABLE)?;
        io.feature(&intensity(100))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&led_color(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, Cyborg)
}

pub static DETECTORS: &[Detector] = &[det(
    "MadCatz Cyborg Gaming Light",
    0x06A3,
    0x0DC5,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // Initialize, SetIntensity(100) from the RGBController constructor, then SetLEDColor
    #[test]
    fn enable_intensity_colour() {
        let [first, _, next] = pushes(Cyborg, 1, vec![], &[Color(1, 2, 3)], &[Color(9, 8, 7)]);
        assert_eq!(
            first,
            vec![
                Op::F(vec![0xA1, 0]),
                Op::F(vec![0xA6, 0, 100]),
                Op::F(vec![0xA2, 0, 1, 2, 3, 0, 0, 0, 0])
            ]
        );
        assert_eq!(next, vec![Op::F(vec![0xA2, 0, 9, 8, 7, 0, 0, 0, 0])]);
        assert_eq!(intensity(200), [0xA6, 0, 100]);
    }
}
