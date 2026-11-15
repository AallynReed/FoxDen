//! Ported from OpenRGB Controllers/N5312AController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "n5312a";
const REPORT: u8 = 0x07;
const DIRECT: u8 = 0x02;
const BRIGHTNESS_MAX: u8 = 0x64;

pub const INIT: [u8; 8] = [REPORT, 0xA0, 0, 0, 0, 0, 0, 0];

pub fn set_color(c: Color) -> [u8; 8] {
    [REPORT, 0x0B, 1, c.0, c.1, c.2, 0, 0]
}

pub fn set_mode(mode: u8, brightness: u8, speed: u8) -> [u8; 8] {
    [REPORT, 0x0A, mode, 1, 1, 1, speed, brightness]
}

#[derive(Default)]
struct N5312a {
    moded: bool,
}

impl Proto for N5312a {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&INIT)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&set_color(colors[0]))?;
        if !self.moded {
            io.feature(&set_mode(DIRECT, BRIGHTNESS_MAX, 0))?;
            self.moded = true;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, N5312a::default())
}

pub static DETECTORS: &[Detector] = &[det(
    "N5312A USB Optical Mouse",
    0x4E53,
    0x5406,
    1,
    0xFF01,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SendInit from the constructor, SetMode(Direct) then SetColor
    #[test]
    fn init_mode_colour() {
        let [first, _, next] = pushes(
            N5312a::default(),
            1,
            vec![],
            &[Color(1, 2, 3)],
            &[Color(4, 5, 6)],
        );
        assert_eq!(
            first,
            vec![
                Op::F(vec![7, 0xA0, 0, 0, 0, 0, 0, 0]),
                Op::F(vec![7, 0x0B, 1, 1, 2, 3, 0, 0]),
                Op::F(vec![7, 0x0A, 2, 1, 1, 1, 0, 0x64]),
            ]
        );
        assert_eq!(next, vec![Op::F(vec![7, 0x0B, 1, 4, 5, 6, 0, 0])]);
    }
}
