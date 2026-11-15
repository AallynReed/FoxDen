//! Ported from OpenRGB Controllers/AOCMouseController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "aoc-mouse";

/// SendDirect: static single colour, high brightness, medium speed, clockwise; logo then wheel.
pub fn direct(logo: Color, wheel: Color) -> [u8; 60] {
    let mut b = [0u8; 60];
    b[..10].copy_from_slice(&[0x20, 0x03, 0x01, 0x00, 0x02, 0x03, 0x00, 0x01, 0x02, 0xFF]);
    let set = [
        (0x0D, 0x01),
        (0x0E, 0x03),
        (0x0F, 0xFF),
        (0x10, 0x7F),
        (0x13, 0x01),
        (0x14, 0x04),
        (0x17, 0xFF),
        (0x19, 0x01),
        (0x1A, 0x05),
        (0x1C, 0xFF),
        (0x1F, 0x01),
        (0x20, 0x06),
        (0x21, 0xFF),
        (0x23, 0xFF),
        (0x25, 0x01),
        (0x26, 0x07),
        (0x27, 0xFF),
        (0x28, 0xFF),
        (0x2C, 0x0A),
        (0x2D, 0x0A),
        (0x30, 0x14),
        (0x31, 0x08),
        (0x3A, 0x32),
        (0x3B, 0x32),
    ];
    for (at, v) in set {
        b[at] = v;
    }
    b[0x33..0x36].copy_from_slice(&rgb(logo));
    b[0x36..0x39].copy_from_slice(&rgb(wheel));
    b
}

struct Mouse;

impl Proto for Mouse {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&direct(colors[0], colors[1]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(info, FAMILY, d, DeviceKind::Mouse, 1, 3, &[(0, 2), (0, 0)]);
    boxed(api, info, FAMILY, dev, Mouse)
}

pub static DETECTORS: &[Detector] = &[det("AOC GM500", 0x3938, 0x1179, 1, 0xFF19, 0xFF19, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // AOCMouseController::SendDirect -> SendPacket
    #[test]
    fn direct_report() {
        let a = [Color(1, 2, 3), Color(4, 5, 6)];
        let [first, _, _] = pushes(Mouse, 2, vec![], &a, &[]);
        let Op::F(p) = &first[0] else { panic!() };
        assert_eq!(first.len(), 1);
        assert_eq!(p.len(), 60);
        assert_eq!(p[..10], [0x20, 3, 1, 0, 2, 3, 0, 1, 2, 0xFF]);
        assert_eq!(p[0x0D..0x11], [1, 3, 0xFF, 0x7F]);
        assert_eq!(p[0x2C..0x32], [0x0A, 0x0A, 0, 0, 0x14, 0x08]);
        assert_eq!(p[0x32..0x3C], [0, 1, 2, 3, 4, 5, 6, 0, 0x32, 0x32]);
        assert_eq!(p.iter().filter(|&&v| v == 0xFF).count(), 8);
    }
}
