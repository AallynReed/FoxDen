//! Ported from OpenRGB Controllers/EKController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "ek-loop";

const STATIC_ROW: [u8; 16] = [
    0x10, 0x12, 0x29, 0xAA, 0x01, 0x10, 0xA2, 0x60, 0x00, 0x10, 0x20, 0x01, 0x01, 0x00, 0xFF, 0x64,
];

/// SendUpdate in static mode, whose speed table row is all zero.
pub fn update(c: Color) -> [u8; 63] {
    let mut b = [0u8; 63];
    b[..16].copy_from_slice(&STATIC_ROW);
    b[16..19].copy_from_slice(&rgb(c));
    b[14] = 0;
    b[10] = 0x10;
    b[47] = 0xFF;
    b
}

struct Ek;

impl Proto for Ek {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&update(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, Ek)
}

pub static DETECTORS: &[Detector] = &[det("EK Loop Connect", 0x0483, 0x5750, 0, 0xFFA0, 1, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // EKController::SetColor -> SendUpdate with mode static
    #[test]
    fn static_colour() {
        let [first, _, _] = pushes(Ek, 1, vec![], &[Color(1, 2, 3)], &[]);
        let mut e = pad(
            &[
                0x10, 0x12, 0x29, 0xAA, 0x01, 0x10, 0xA2, 0x60, 0x00, 0x10, 0x10, 0x01, 0x01, 0x00,
                0x00, 0x64, 1, 2, 3,
            ],
            63,
        );
        e[47] = 0xFF;
        assert_eq!(first, vec![w(e)]);
    }
}
