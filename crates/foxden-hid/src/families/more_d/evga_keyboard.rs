// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "evga-keyboard";
const VID: u16 = 0x3842;
const ID_6_SIZE: usize = 792;
const ID_7_SIZE: usize = 136;
const FULL_SIZE: usize = 108;
const Z20_EXTRA: usize = 24;

#[rustfmt::skip]
const PACKET_MAP: [u8; FULL_SIZE + Z20_EXTRA] = [
      1,   2,   3,   4,   5,   6,   7,   8,   9,  10,
     11,  12,  13,  14,  15,  16,  22,  23,  24,  25,
     26,  27,  28,  29,  30,  31,  32,  33,  34,  35,
     36,  37,  38,  44,  45,  46,  47,  48,  49,  50,
     51,  52,  53,  54,  55,  56,  57,  58,  59,  60,
     66,  67,  68,  69,  70,  71,  72,  73,  74,  75,
     76,  77,  78,  83,  84,  85,  86,  87,  88,  89,
     90,  91,  92,  93,  94,  96, 103, 104, 105, 106,
    107, 108, 109, 110, 111, 112, 113,  39,  40,  41,
     42,  64, 101,  98,  99, 100,  79,  80,  81,  61,
     62,  63, 114, 115,  18,  19,  20, 118, 176, 177,
    178, 179, 180, 181, 182, 183, 184, 160, 161, 162,
    163, 164, 165, 166, 167, 168,   0,  21,  43,  65,
     82, 102,
];

#[rustfmt::skip]
const FULL_MATRIX: [u32; 6 * 21] = [
     0, NA,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, 104, 105, 106, 107,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,  87,  88,  89,  90,
    33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,  99, 100, 101,  91,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, NA, NA, NA, NA,  96,  97,  98,  NA,
    63, NA, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, NA, NA, 75, NA,  93,  94,  95,  92,
    76, 77, 78, NA, NA, NA, 79, NA, NA, NA, 80, 81, 82, 83, 84, 85, 86, 102,  NA, 103,  NA,
];

/// GetChecksum: zero minus every byte from the zone byte on.
fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |sum, b| sum.wrapping_sub(*b))
}

/// SetLedsDirect's feature report.
pub fn direct_report(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; ID_6_SIZE];
    b[..4].copy_from_slice(&[0x06, 0xEA, 0x02, 0x01]);
    for (c, &slot) in colors.iter().zip(PACKET_MAP.iter()) {
        let at = 8 + slot as usize * 4;
        b[at..at + 4].copy_from_slice(&[0xFF, c.0, c.1, c.2]);
    }
    b[7] = checksum(&b[8..]);
    b
}

/// SetSleepTime with zero minutes, sent by the constructor.
pub fn sleep_reports() -> [Vec<u8>; 2] {
    let mut a = vec![0u8; ID_7_SIZE];
    a[..9].copy_from_slice(&[0x07, 0xEA, 0x02, 0x1B, 0x00, 0x00, 0x00, 0xFE, 0x02]);
    let mut b = vec![0u8; ID_7_SIZE];
    b[..4].copy_from_slice(&[0x07, 0xEA, 0x02, 0x03]);
    b[7] = checksum(&b[8..]);
    [a, b]
}

pub struct Keyboard;

impl Proto for Keyboard {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for r in sleep_reports() {
            io.feature(&r)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&direct_report(colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let n = if matches!(info.pid, 0x260A | 0x2610) {
        FULL_SIZE + Z20_EXTRA
    } else {
        FULL_SIZE
    };
    let dev = mapped(
        hid_id(info, FAMILY),
        d.name,
        DeviceKind::Keyboard,
        &FULL_MATRIX,
        21,
        n,
    );
    Ok(boxed(Box::new(info.open(api)?), dev, FAMILY, Keyboard))
}

pub static DETECTORS: &[Detector] = &[
    det("EVGA Z15 Keyboard", VID, 0x260E, 1, 0x08, 0x4B, open),
    det("EVGA Z15 Keyboard", VID, 0x2608, 1, 0x08, 0x4B, open),
    det("EVGA Z20 Keyboard", VID, 0x260A, 1, 0x08, 0x4B, open),
    det("EVGA Z20 Keyboard", VID, 0x2610, 1, 0x08, 0x4B, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetSleepTime from the constructor, then SetLedsDirect
    #[test]
    fn sleep_time_then_direct() {
        let mut a = vec![Color::BLACK; 132];
        a[0] = Color(1, 2, 3);
        a[131] = Color(4, 5, 6);
        let [first, _, _] = pushes(Keyboard, 132, vec![], &a, &[]);
        assert_eq!(
            first[0],
            Op::F(pad(&[0x07, 0xEA, 0x02, 0x1B, 0, 0, 0, 0xFE, 0x02], 136))
        );
        assert_eq!(first[1], Op::F(pad(&[0x07, 0xEA, 0x02, 0x03], 136)));
        let mut d = pad(&[0x06, 0xEA, 0x02, 0x01], 792);
        for slot in 0..132usize {
            d[8 + PACKET_MAP[slot] as usize * 4] = 0xFF;
        }
        d[8 + 4..8 + 8].copy_from_slice(&[0xFF, 1, 2, 3]);
        d[8 + 102 * 4..8 + 103 * 4].copy_from_slice(&[0xFF, 4, 5, 6]);
        let sum: u32 = d[8..].iter().map(|&b| b as u32).sum();
        d[7] = (0u32.wrapping_sub(sum) & 0xFF) as u8;
        assert_eq!(first[2], Op::F(d));
    }
}
