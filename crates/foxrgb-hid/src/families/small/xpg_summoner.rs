//! Ported from OpenRGB Controllers/XPGSummonerKeyboardController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "xpg-summoner";
const BRIGHTNESS: u8 = 0x64;

/// Hardware slot of each key, row by row; LED order is the order of the non-NA cells.
const MATRIX: [[u32; 21]; 6] = [
    [
        11, NA, 22, 30, 25, 27, 7, 51, 57, 62, 86, 87, 83, 85, 79, 72, 0, NA, NA, NA, NA,
    ],
    [
        14, 15, 23, 31, 39, 38, 46, 47, 55, 63, 71, 70, 54, 81, 102, 118, 110, 92, 100, 108, 109,
    ],
    [
        9, 8, 16, 24, 32, 33, 41, 40, 48, 56, 64, 65, 49, 82, 94, 119, 111, 88, 96, 104, 112,
    ],
    [
        17, 10, 18, 26, 34, 35, 43, 42, 50, 58, 66, 67, NA, 84, NA, NA, NA, 89, 97, 105, NA,
    ],
    [
        121, NA, 12, 20, 28, 36, 37, 45, 44, 52, 60, 69, 122, NA, NA, 115, NA, 90, 98, 106, 114,
    ],
    [
        6, 124, 75, NA, NA, NA, 91, NA, NA, NA, 77, 125, 61, 4, 117, 93, 101, 99, NA, 107, NA,
    ],
];

fn keys() -> impl Iterator<Item = (u32, u32, usize)> {
    MATRIX.iter().enumerate().flat_map(|(r, row)| {
        row.iter()
            .enumerate()
            .filter(|(_, v)| **v != NA)
            .map(move |(c, &v)| (c as u32, r as u32, v as usize))
    })
}

pub fn packet(head: [u8; 6], data: &[u8]) -> [u8; 265] {
    let mut p = [0u8; 265];
    p[..6].copy_from_slice(&head);
    p[6..6 + data.len()].copy_from_slice(data);
    p
}

pub fn init() -> [u8; 265] {
    packet([0x07, 0xEA, 0, 0, 0, 0], &[])
}

/// DeviceUpdateLEDs' 126 x 4 frame, split into SendColorDataPacket blocks of 256 bytes.
pub fn colour_packets(colors: &[Color]) -> [[u8; 265]; 2] {
    let mut frame = [0u8; 126 * 4];
    for ((_, _, slot), c) in keys().zip(colors) {
        frame[slot * 4..slot * 4 + 4].copy_from_slice(&[BRIGHTNESS, c.0, c.1, c.2]);
    }
    [
        packet([0x07, 0xA3, 0x08, 0, 0, 0], &frame[..256]),
        packet([0x07, 0xA3, 0x08, 0, 1, 0], &frame[256..]),
    ]
}

struct Summoner;

impl Proto for Summoner {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&init())?;
        io.sleep(2);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in colour_packets(colors) {
            io.write(&p)?;
            io.sleep(2);
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let at: Vec<(u32, u32)> = keys().map(|(c, r, _)| (c, r)).collect();
    let dev = cells(info, FAMILY, d, DeviceKind::Keyboard, 21, 6, &at);
    boxed(api, info, FAMILY, dev, Summoner)
}

pub static DETECTORS: &[Detector] = &[det(
    "XPG Summoner Gaming Keyboard",
    0x125F,
    0x9418,
    2,
    0xFF01,
    0x0001,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;

    const LEDS: usize = 104;
    use rec::*;

    // SendInitialize from the constructor, then DeviceUpdateLEDs -> SendColors
    #[test]
    fn init_then_two_blocks() {
        assert_eq!(keys().count(), LEDS);
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3); // Escape, slot 11
        a[103] = Color(4, 5, 6); // last key, slot 107
        let [first, _, _] = pushes(Summoner, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 6);
        assert_eq!(first[0], w(pad(&[0x07, 0xEA], 265)));
        assert_eq!(first[1], Op::Sleep(2));
        let Op::W(p0) = &first[2] else { panic!() };
        let Op::W(p1) = &first[4] else { panic!() };
        assert_eq!(p0[..6], [7, 0xA3, 8, 0, 0, 0]);
        assert_eq!(p1[..6], [7, 0xA3, 8, 0, 1, 0]);
        assert_eq!(p0[6 + 44..6 + 48], [0x64, 1, 2, 3]);
        assert_eq!(p1[6 + 428 - 256..6 + 432 - 256], [0x64, 4, 5, 6]);
        assert!(p1[6 + 248..].iter().all(|&b| b == 0));
        // every key gets the brightness byte even when black
        assert_eq!(
            p0.iter().filter(|&&b| b == 0x64).count() + p1.iter().filter(|&&b| b == 0x64).count(),
            LEDS
        );
    }
}
