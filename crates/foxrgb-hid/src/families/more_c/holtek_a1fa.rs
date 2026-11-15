//! Ported from OpenRGB Controllers/HoltekController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "holtek-a1fa";

struct A1fa;

impl Proto for A1fa {
    // SendData for Static: brightness 0x20, speed 0, no random preset
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let [r, g, b] = rgb(colors[0]);
        io.feature(&[0x00, 0x08, 0x00, 0x20, 0x00, 0x00, r, g, b])
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, A1fa)
}

pub static DETECTORS: &[Detector] = &[det(
    "Holtek Mousemat",
    0x04D9,
    0xA1FA,
    2,
    0xFF00,
    0xFF00,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn static_packet() {
        let [first, _, next] = pushes(A1fa, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        assert_eq!(first, vec![Op::F(vec![0, 8, 0, 0x20, 0, 0, 1, 2, 3])]);
        assert_eq!(next, vec![Op::F(vec![0, 8, 0, 0x20, 0, 0, 4, 5, 6])]);
    }
}
