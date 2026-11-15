//! Ported from OpenRGB Controllers/GameSirController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "gamesir";

pub fn set_color(c: Color) -> [u8; 65] {
    let mut buf = [0u8; 65];
    buf[1..10].copy_from_slice(&[0x05, 0x08, 0x0A, 0x01, 0x03, c.0, c.1, c.2, 0x00]);
    buf[10] = buf[1..10].iter().map(|&b| b as u32).sum::<u32>() as u8;
    buf
}

struct GameSir;

impl Proto for GameSir {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&set_color(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, GameSir)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "GameSir Nova 2 Lite",
        0x3537,
        0x100F,
        2,
        0xFF7A,
        0x0001,
        open,
    ),
    det(
        "GameSir Nova 2 Lite (Wireless)",
        0x3537,
        0x1098,
        2,
        0xFF7A,
        0x0001,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // GameSirController::SetColor
    #[test]
    fn colour_report() {
        let [first, _, next] = pushes(
            GameSir,
            1,
            vec![],
            &[Color(1, 2, 3)],
            &[Color(255, 255, 255)],
        );
        let mut e = pad(&[0, 0x05, 0x08, 0x0A, 0x01, 0x03, 1, 2, 3, 0, 0x21], 65);
        assert_eq!(first, vec![w(e.clone())]);
        e[6..9].copy_from_slice(&[255, 255, 255]);
        e[10] = (0x05 + 0x08 + 0x0A + 0x01 + 0x03 + 255 * 3u32) as u8;
        assert_eq!(next, vec![w(e)]);
    }
}
