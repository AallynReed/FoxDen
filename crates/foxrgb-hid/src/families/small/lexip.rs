//! Ported from OpenRGB Controllers/LexipMouseController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "lexip";

pub fn set_direct(c: Color) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[1..9].copy_from_slice(&[0x24, 0x01, c.0, c.1, c.2, 0x00, 0x64, 0x80]);
    b
}

struct Lexip;

impl Proto for Lexip {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&set_direct(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Lexip)
}

pub static DETECTORS: &[Detector] = &[det(
    "Np93 ALPHA - Gaming Mouse",
    0x04D8,
    0xFD0A,
    0,
    0x0001,
    2,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // LexipMouseController::SetDirect
    #[test]
    fn direct_report() {
        let [first, _, next] = pushes(Lexip, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        assert_eq!(
            first,
            vec![w(pad(&[0, 0x24, 0x01, 1, 2, 3, 0, 0x64, 0x80], 64))]
        );
        assert_eq!(
            next,
            vec![w(pad(&[0, 0x24, 0x01, 4, 5, 6, 0, 0x64, 0x80], 64))]
        );
    }
}
