// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-eve1800";
const LEDS: usize = 10;

struct Eve;

impl Proto for Eve {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        let mut b = [0u8; 65];
        b[..5].copy_from_slice(&[0x40, 0x01, 0x00, 0x00, 0xFF]);
        io.write(&b)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut init = [0u8; 65];
        init[..3].copy_from_slice(&[0x44, 0x01, 0x04]);
        io.write(&init)?;
        let mut b = [0u8; 65];
        b[..2].copy_from_slice(&[0x44, 0x02]);
        for (i, c) in colors.iter().take(20).enumerate() {
            b[4 + i * 3..7 + i * 3].copy_from_slice(&rgb(*c));
        }
        io.write(&b)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Keyboard, LEDS);
    boxed(api, info, FAMILY, dev, Eve)
}

pub static DETECTORS: &[Detector] = &[det(
    "HyperX Eve 1800 (HP)",
    0x03F0,
    0x08C2,
    2,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn brightness_then_frame() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[9] = Color(4, 5, 6);
        let [first, _, _] = pushes(Eve, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 3);
        assert_eq!(first[0], w(pad(&[0x40, 0x01, 0, 0, 0xFF], 65)));
        assert_eq!(first[1], w(pad(&[0x44, 0x01, 0x04, 0], 65)));
        let mut p = pad(&[0x44, 0x02, 0, 0, 1, 2, 3], 65);
        p[31..34].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[2], w(p));
    }
}
