// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-alloy-elite";
const LEDS: usize = 128;

pub const KEYS: [usize; 106] = [
    0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x20, 0x21, 0x22, 0x23, 0x24, 0x26, 0x27, 0x28, 0x29,
    0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x37, 0x38, 0x39, 0x3A, 0x3B,
    0x3C, 0x3E, 0x3F, 0x41, 0x44, 0x45, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x51, 0x54,
    0x55, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5E, 0x5F, 0x61, 0x64, 0x65, 0x68, 0x69, 0x6A, 0x6B, 0x6C,
    0x6E, 0x6F, 0x74, 0x75, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E, 0x7F, 0x81, 0x84, 0x85, 0x88,
    0x89, 0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F, 0x91, 0x94, 0x95,
];

const EXT_RED: [usize; 22] = [
    0x08, 0x48, 0x88, 0x09, 0x89, 0x0A, 0x8A, 0x0B, 0x8B, 0x0C, 0x8C, 0x0D, 0x8D, 0x0E, 0x8F, 0x8E,
    0x0F, 0x4F, 0x92, 0x13, 0x93, 0x12,
];
const EXT_GRN: [usize; 22] = [
    0x29, 0x28, 0x78, 0x19, 0x79, 0x1A, 0x7A, 0x1B, 0x7B, 0x1C, 0x7C, 0x1D, 0x7D, 0x1E, 0x6E, 0x7E,
    0x1F, 0x6F, 0x82, 0x23, 0x83, 0x22,
];
const EXT_BLU: [usize; 22] = [
    0x39, 0x38, 0x68, 0x3A, 0x69, 0x2A, 0x6A, 0x2B, 0x6B, 0x2C, 0x6C, 0x2D, 0x6D, 0x2E, 0x5E, 0x5D,
    0x2F, 0x5F, 0x72, 0x33, 0x73, 0x32,
];

pub const MATRIX: [&[u32]; 6] = [
    &[
        0, NA, 16, 30, 44, 54, NA, 65, 75, 84, 95, NA, 8, 23, 38, 6, 22, 36, 49, NA, NA, NA, NA,
    ],
    &[
        1, 17, 31, 45, 55, 66, 76, 85, 96, 9, 24, NA, 39, 7, 37, NA, 60, 70, 80, 52, 63, 73, 82,
    ],
    &[
        2, NA, 18, 32, 46, 56, NA, 67, 77, 86, 97, 10, 25, 40, 90, 101, 50, 61, 71, 51, 62, 72, 93,
    ],
    &[
        3, NA, 19, 33, 47, 57, NA, 68, 78, 87, 98, 11, 26, 41, 28, 14, NA, NA, NA, 92, 103, 53, NA,
    ],
    &[
        4, 20, 34, 48, 58, 69, NA, 79, NA, 88, 99, 12, 27, 42, 81, NA, NA, 102, NA, 64, 74, 83, 104,
    ],
    &[
        5, 21, 35, NA, NA, NA, NA, 59, NA, NA, NA, NA, 89, 100, 13, 91, 15, 29, 43, 94, NA, 105, NA,
    ],
];

// 18-LED light bar and the 4 media keys, placed above the keys
const TOP: [u32; 23] = [
    106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, NA,
    124, 125, 126, 127,
];
const FULL_MAP: [&[u32]; 7] = [
    &TOP, MATRIX[0], MATRIX[1], MATRIX[2], MATRIX[3], MATRIX[4], MATRIX[5],
];

/// SendDirect: one colour channel of the 106 keys.
fn direct(channel: u8, values: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[..4].copy_from_slice(&[0x07, 0x16, channel, 0xA0]);
    for (&k, &v) in KEYS.iter().zip(values) {
        b[k] = v;
    }
    b
}

/// The red, green and blue SendDirect reports for the first 106 colours.
pub fn planes(colors: &[Color]) -> [Vec<u8>; 3] {
    let ch = |f: fn(&Color) -> u8| colors.iter().take(106).map(f).collect::<Vec<u8>>();
    [
        direct(0x01, &ch(|c| c.0)),
        direct(0x02, &ch(|c| c.1)),
        direct(0x03, &ch(|c| c.2)),
    ]
}

/// SendDirectExtended: the 22 light bar and media LEDs, scattered over bytes 0x08..0x94.
fn extended(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[..4].copy_from_slice(&[0x07, 0x16, 0x04, 0xA0]);
    for (i, c) in colors.iter().skip(106).take(22).enumerate() {
        b[EXT_RED[i]] = c.0;
        b[EXT_GRN[i]] = c.1;
        b[EXT_BLU[i]] = c.2;
    }
    b
}

/// SetMode(static) as DeviceUpdateMode sends it on entering Direct.
fn static_effect() -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[..3].copy_from_slice(&[0x07, 0x02, 0x01]);
    b[0x09] = 0x01;
    b[0x0A] = 0x01;
    b[0x1B..=0x22].fill(0xFF);
    b[0x6B..0x73].copy_from_slice(&[0x09, 0x09, 0x05, 0x05, 0x06, 0x05, 0x00, 0x09]);
    b[0x73..=0x7A].fill(0x02);
    b
}

struct Elite;

impl Proto for Elite {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&static_effect())?;
        io.sleep(100);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in planes(colors) {
            io.feature(&p)?;
            io.sleep(5);
        }
        io.feature(&extended(colors))
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &FULL_MAP, LEDS);
    boxed(api, info, FAMILY, dev, Elite)
}

pub static DETECTORS: &[Detector] = &[det(
    "HyperX Alloy Elite RGB",
    0x0951,
    0x16BE,
    2,
    0xFF01,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn tables_match_openrgb() {
        assert_eq!(KEYS.len(), 106);
        let mut seen: Vec<u32> = FULL_MAP.iter().flat_map(|r| r.iter().copied()).collect();
        seen.retain(|&v| v != NA);
        seen.sort();
        assert_eq!(seen, (0..LEDS as u32).collect::<Vec<_>>());
    }

    #[test]
    fn effect_then_planes() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[105] = Color(4, 5, 6);
        a[106] = Color(7, 8, 9);
        a[127] = Color(10, 11, 12);
        let [first, _, _] = pushes(Elite, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 9);
        let mut e = vec![0u8; 264];
        e[0] = 0x07;
        e[1] = 0x02;
        e[2] = 0x01;
        e[0x09] = 0x01;
        e[0x0A] = 0x01;
        e[0x1B..=0x22].fill(0xFF);
        e[0x6B] = 0x09;
        e[0x6C] = 0x09;
        e[0x6D] = 0x05;
        e[0x6E] = 0x05;
        e[0x6F] = 0x06;
        e[0x70] = 0x05;
        e[0x72] = 0x09;
        e[0x73..=0x7A].fill(0x02);
        assert_eq!(first[0], Op::F(e));
        assert_eq!(first[1], Op::Sleep(100));
        for (n, ch) in [1u8, 2, 3].into_iter().enumerate() {
            let mut p = pad(&[0x07, 0x16, ch, 0xA0], 264);
            p[0x08] = ch;
            p[0x95] = ch + 3;
            assert_eq!(first[2 + n * 2], Op::F(p));
            assert_eq!(first[3 + n * 2], Op::Sleep(5));
        }
        let mut x = pad(&[0x07, 0x16, 0x04, 0xA0], 264);
        x[0x08] = 7;
        x[0x29] = 8;
        x[0x39] = 9;
        x[0x12] = 10;
        x[0x22] = 11;
        x[0x32] = 12;
        assert_eq!(first[8], Op::F(x));
    }
}
