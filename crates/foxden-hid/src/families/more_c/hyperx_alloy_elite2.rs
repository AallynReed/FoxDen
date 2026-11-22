// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::hyperx_origins::{send_direct, with_gaps};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-alloy-elite2";
const LEDS: usize = 128;
const SKIP: [usize; 16] = [
    23, 29, 41, 47, 70, 71, 76, 77, 87, 88, 93, 99, 100, 102, 108, 113,
];

const MATRIX: [&[u32]; 8] = [
    &[
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 104, 107, 108, 98, NA, NA,
    ],
    &[
        110, 111, 112, 113, NA, 114, 115, 116, NA, 117, 118, 119, 120, NA, 121, 122, 123, NA, 124,
        125, 126, 127,
    ],
    &[
        0, 12, 18, 23, 28, NA, 34, 39, 44, 50, 56, 62, 66, 70, NA, 76, 80, 85, NA, NA, NA, NA,
    ],
    &[
        1, 7, 13, 19, 24, 29, 35, 40, 45, 51, 57, 63, 67, 71, NA, 77, 81, 86, 89, 94, 99, 105,
    ],
    &[
        2, NA, 8, 14, 20, 25, 30, 36, 41, 46, 52, 58, 64, 68, 72, 78, 82, 87, 90, 95, 100, 106,
    ],
    &[
        3, NA, 9, 15, 21, 26, 31, 37, 42, 47, 53, 59, 65, 69, 73, NA, NA, NA, 91, 96, 101, NA,
    ],
    &[
        4, 6, NA, 10, 16, 22, 27, 32, 38, 43, 48, 54, 60, 74, NA, NA, 83, NA, 92, 97, 102, 109,
    ],
    &[
        5, 11, 17, NA, NA, NA, NA, 33, NA, NA, NA, 49, 61, 55, 75, 79, 84, 88, 93, NA, 103, NA,
    ],
];

struct Elite2;

impl Proto for Elite2 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send_direct(io, 0x00, &with_gaps(colors, &SKIP), 9)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MATRIX, LEDS);
    boxed(api, info, FAMILY, dev, Elite2)
}

pub static DETECTORS: &[Detector] = &[
    det("HyperX Alloy Elite 2", 0x0951, 0x1711, 3, 0xFF90, ANY, open),
    det(
        "HyperX Alloy Elite 2 (HP)",
        0x03F0,
        0x058F,
        3,
        0xFF90,
        ANY,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout_matches_openrgb() {
        assert_eq!(MATRIX.iter().map(|r| r.len()).sum::<usize>(), 8 * 22);
        let mut seen: Vec<u32> = MATRIX.iter().flat_map(|r| r.iter().copied()).collect();
        seen.retain(|&v| v != NA);
        seen.sort();
        assert_eq!(seen, (0..LEDS as u32).collect::<Vec<_>>());
    }

    #[test]
    fn init_then_packets() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[23] = Color(4, 5, 6); // slot 24: packet 1, colour 8
        a[127] = Color(7, 8, 9); // slot 143: packet 8, colour 15
        let [first, _, _] = pushes(Elite2, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 10);
        assert_eq!(first[0], Op::F(pad(&[0, 0x04, 0xF2], 65)));
        let full = || {
            let mut p = vec![0u8; 65];
            for i in 0..16 {
                p[i * 4 + 1] = 0x81;
            }
            p
        };
        let mut p0 = full();
        p0[2..5].copy_from_slice(&[1, 2, 3]);
        assert_eq!(first[1], Op::F(p0));
        let mut p1 = full();
        p1[8 * 4 + 2..8 * 4 + 5].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[2], Op::F(p1));
        assert_eq!(first[5], Op::F(full()));
        let mut p8 = full();
        p8[15 * 4 + 2..15 * 4 + 5].copy_from_slice(&[7, 8, 9]);
        assert_eq!(first[9], Op::F(p8));
    }
}
