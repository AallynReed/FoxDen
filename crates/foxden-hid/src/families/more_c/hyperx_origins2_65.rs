//! Ported from OpenRGB Controllers/HyperXKeyboardController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-origins2-65";
const LEDS: usize = 74;

const MATRIX: [&[u32]; 5] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
    &[15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, NA, 29],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44],
    &[45, 61, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 59],
    &[60, 62, 63, 65, NA, NA, 66, NA, NA, 68, 69, 70, 71, 72, 73],
];

struct Origins2;

impl Proto for Origins2 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut init = [0u8; 65];
        init[..3].copy_from_slice(&[0x44, 0x01, 0x04]);
        io.write(&init)?;
        let mut chunks = colors.chunks(20);
        for seq in 0..4u8 {
            let mut b = [0u8; 65];
            b[..3].copy_from_slice(&[0x44, 0x02, seq]);
            for (i, c) in chunks.next().unwrap_or(&[Color::BLACK]).iter().enumerate() {
                b[4 + i * 3..7 + i * 3].copy_from_slice(&rgb(*c));
            }
            io.write(&b)?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MATRIX, LEDS);
    boxed(api, info, FAMILY, dev, Origins2)
}

pub static DETECTORS: &[Detector] = &[det(
    "HyperX Origins 2 65 (HP)",
    0x03F0,
    0x0CC2,
    3,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn init_then_four_packets() {
        assert_eq!(MATRIX.iter().map(|r| r.len()).sum::<usize>(), 5 * 15);
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[73] = Color(4, 5, 6); // packet 3, colour 13
        let [first, _, _] = pushes(Origins2, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 5);
        assert_eq!(first[0], w(pad(&[0x44, 0x01, 0x04, 0], 65)));
        assert_eq!(first[1], w(pad(&[0x44, 0x02, 0, 0, 1, 2, 3], 65)));
        assert_eq!(first[2], w(pad(&[0x44, 0x02, 1], 65)));
        assert_eq!(first[3], w(pad(&[0x44, 0x02, 2], 65)));
        let mut p3 = pad(&[0x44, 0x02, 3], 65);
        p3[43..46].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[4], w(p3));
    }
}
