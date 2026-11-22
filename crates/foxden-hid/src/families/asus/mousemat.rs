// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-mousemat";
const VID: u16 = 0x0B05;

/// UpdateLeds: 15 colours, four bytes each.
pub fn packet(colors: &[Color]) -> [u8; 65] {
    let mut b = report(&[0xEE, 0xC0, 0x81, 0x00, 0x00]);
    for (i, c) in colors[..15].iter().enumerate() {
        b[6 + i * 4..9 + i * 4].copy_from_slice(&rgb(*c));
    }
    b
}

struct Mat;

impl Proto for Mat {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&packet(colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 15);
    boxed(api, info, FAMILY, dev, Mat)
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS ROG Balteus", VID, 0x1891, ANY, 0xFF06, 1, open),
    det("ASUS ROG Balteus Qi", VID, 0x1890, ANY, 0xFF06, 1, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AuraMousemat::DeviceUpdateLEDs
    #[test]
    fn frame_packet() {
        let mut a = vec![Color::BLACK; 15];
        a[0] = Color(1, 2, 3);
        a[14] = Color(4, 5, 6);
        let [first, _, _] = pushes(Mat, 15, vec![], &a, &[]);
        let mut p = pad(&[0xEE, 0xC0, 0x81, 0, 0, 0, 1, 2, 3], 65);
        p[62..65].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first, vec![w(p)]);
    }
}
