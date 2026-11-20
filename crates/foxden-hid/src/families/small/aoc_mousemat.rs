//! Ported from OpenRGB Controllers/AOCMousematController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "aoc-mousemat";

/// SendDirect: static single colour, high brightness, medium speed, clockwise.
pub fn direct(c: Color) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..6].copy_from_slice(&[0x20, 0x03, 0x02, 0x00, 0x01, 0x00]);
    for at in [0x09, 0x0C, 0x0F, 0x10, 0x13, 0x17, 0x19, 0x1A, 0x1B, 0x1D] {
        b[at] = 0xFF;
    }
    b[0x0D] = 0x3F;
    b[0x1E] = 0x32;
    b[0x1F] = 0x32;
    b[6..9].copy_from_slice(&rgb(c));
    b
}

struct Mousemat;

impl Proto for Mousemat {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&direct(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, Mousemat)
}

pub static DETECTORS: &[Detector] = &[det(
    "AOC AGON AMM700",
    0x3938,
    0x1162,
    1,
    0xFF19,
    0xFF19,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // AOCMousematController::SendDirect -> SendPacket
    #[test]
    fn direct_report() {
        let [first, _, _] = pushes(Mousemat, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        let e = vec![
            0x20, 0x03, 0x02, 0x00, 0x01, 0x00, 1, 2, 3, 0xFF, 0, 0, 0xFF, 0x3F, 0, 0xFF, 0xFF, 0,
            0, 0xFF, 0, 0, 0, 0xFF, 0, 0xFF, 0xFF, 0xFF, 0, 0xFF, 0x32, 0x32,
        ];
        assert_eq!(first, vec![Op::F(e)]);
    }
}
