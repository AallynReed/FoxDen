//! Ported from OpenRGB Controllers/HoltekController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "holtek-a070";

struct A070;

impl Proto for A070 {
    // SendMode(Static); it saves, so only once
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&[0x07, 0x0B, 0x01, 0x01, 0, 0, 0, 0])
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let [r, g, b] = rgb(colors[0]);
        io.feature(&[0x07, 0x0A, 0x00, r, g, b, 0, 0])
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, A070)
}

pub static DETECTORS: &[Detector] = &[det(
    "Holtek USB Gaming Mouse",
    0x04D9,
    0xA070,
    1,
    0xFF00,
    2,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn mode_then_colour() {
        let [first, _, next] = pushes(A070, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        assert_eq!(
            first,
            vec![
                Op::F(vec![7, 0x0B, 1, 1, 0, 0, 0, 0]),
                Op::F(vec![7, 0x0A, 0, 1, 2, 3, 0, 0]),
            ]
        );
        assert_eq!(next, vec![Op::F(vec![7, 0x0A, 0, 4, 5, 6, 0, 0])]);
    }
}
