// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "msi-mystic64";

/// SetMode for Static: speed low, brightness level 10, one colour.
pub fn steady(c: Color) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..6].copy_from_slice(&[0x02, 0x00, 0x01, 0x00, 0x0A, 0x01]);
    b[6..9].copy_from_slice(&rgb(c));
    b
}

struct Mystic64;

impl Proto for Mystic64 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&steady(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Keyboard, 1);
    boxed(api, info, FAMILY, dev, Mystic64)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "MSI Mystic Light MS_1562",
        0x1462,
        0x1562,
        ANY,
        0x00FF,
        0x01,
        open,
    ),
    det(
        "MSI Mystic Light MS_1563",
        0x1462,
        0x1563,
        ANY,
        0x00FF,
        0x01,
        open,
    ),
    det(
        "MSI Mystic Light MS_1564",
        0x1462,
        0x1564,
        ANY,
        0x00FF,
        0x01,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn static_packet() {
        let [first, _, next] = pushes(Mystic64, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        assert_eq!(first, vec![Op::F(pad(&[2, 0, 1, 0, 10, 1, 1, 2, 3], 64))]);
        assert_eq!(next, vec![Op::F(pad(&[2, 0, 1, 0, 10, 1, 4, 5, 6], 64))]);
    }
}
