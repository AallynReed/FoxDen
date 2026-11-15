//! Ported from OpenRGB Controllers/A4TechController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "bloody-b820r";
const KEYS: usize = 104;
const PER_PACKET: usize = 58;

const MAP: [&[u32]; 6] = [
    &[
        0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, NA, NA, NA, NA, 13, 14, 15,
    ],
    &[
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
    ],
    &[
        37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    ],
    &[
        58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, NA, NA, NA, NA, 71, 72, 73, NA,
    ],
    &[
        74, NA, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, NA, NA, 86, NA, 87, 88, 89, 90,
    ],
    &[
        91, 92, 93, 94, NA, NA, NA, NA, NA, NA, 95, 96, 97, 98, 99, 100, 101, 102, NA, 103, NA,
    ],
];

/// SetLEDDirect: R1, R2, G1, G2, B1, B2, keys 0..58 in the first half, the rest in the second.
pub fn direct(colors: &[Color]) -> [[u8; 64]; 6] {
    let mut out = [[0u8; 64]; 6];
    for (i, p) in out.iter_mut().enumerate() {
        p[..4].copy_from_slice(&[0x07, 0x03, 0x06, 0x07 + i as u8]);
    }
    for (i, c) in colors.iter().enumerate() {
        let half = i / PER_PACKET;
        let at = 6 + i % PER_PACKET;
        for (ch, v) in rgb(*c).into_iter().enumerate() {
            out[ch * 2 + half][at] = v;
        }
    }
    out
}

struct B820r;

impl Proto for B820r {
    // SendControlPacket(GAIN_CONTROL) from the constructor
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        let mut b = [0u8; 64];
        b[..4].copy_from_slice(&[0x07, 0x03, 0x06, 0x01]);
        io.feature(&b)?;
        b[3] = 0;
        b[8] = 0x01;
        io.feature(&b)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in direct(colors) {
            io.feature(&p)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MAP, KEYS);
    boxed(api, info, FAMILY, dev, B820r)
}

pub static DETECTORS: &[Detector] = &[det("Bloody B820R", 0x09DA, 0xFA10, 2, 0xFF52, 0x0210, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn init_then_direct() {
        assert_eq!(
            MAP.iter()
                .flat_map(|r| r.iter())
                .filter(|&&l| l != NA)
                .count(),
            104
        );
        let mut a = vec![Color::BLACK; 104];
        a[0] = Color(1, 2, 3);
        a[103] = Color(4, 5, 6);
        let [first, _, _] = pushes(B820r, 104, vec![], &a, &[]);
        assert_eq!(first.len(), 8);
        assert_eq!(first[0], Op::F(pad(&[7, 3, 6, 1], 64)));
        assert_eq!(first[1], Op::F(pad(&[7, 3, 6, 0, 0, 0, 0, 0, 1], 64)));
        assert_eq!(first[2], Op::F(pad(&[7, 3, 6, 7, 0, 0, 1], 64)));
        let mut r2 = pad(&[7, 3, 6, 8], 64);
        r2[51] = 4;
        assert_eq!(first[3], Op::F(r2));
        assert_eq!(first[4], Op::F(pad(&[7, 3, 6, 9, 0, 0, 2], 64)));
        let mut b2 = pad(&[7, 3, 6, 0x0C], 64);
        b2[51] = 6;
        assert_eq!(first[7], Op::F(b2));
    }
}
