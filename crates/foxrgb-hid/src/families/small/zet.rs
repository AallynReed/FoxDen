//! Ported from OpenRGB Controllers/ZETKeyboardController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "zet-blade";
const KEYS: usize = 104;
const HEADER_LEN: usize = 9;
const COLOR_LEN: usize = 4;
const DELAY: u64 = 12;
const BRIGHTNESS: u8 = 4;

const SKIP: [usize; 22] = [
    1, 17, 18, 19, 20, 75, 77, 78, 79, 83, 85, 96, 98, 100, 108, 109, 111, 112, 113, 116, 123, 125,
];

const MATRIX: [[u32; 22]; 6] = [
    [
        0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, NA, 13, 14, 15, NA, NA, NA, NA,
    ],
    [
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, NA, 30, 31, 32, 33, 34, 35, 36,
    ],
    [
        37, NA, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    ],
    [
        58, NA, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, NA, NA, NA, NA, 71, 72, 73, NA,
    ],
    [
        74, NA, NA, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, NA, NA, 86, NA, 87, 88, 89, 90,
    ],
    [
        91, 92, 93, NA, NA, NA, NA, 94, NA, NA, NA, 95, 96, 97, 98, 99, 100, 101, 102, NA, 103, NA,
    ],
];

fn header(brightness: u8) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[..9].copy_from_slice(&[0x04, 0xAE, 0x01, 0, 0, 0x1C, 0x02, brightness, 0xFF]);
    b
}

/// SetLEDDirect: key index (with gaps) plus RGB, twelve keys a packet, 0xFF after the last.
pub fn direct_packets(colors: &[Color], brightness: u8) -> Vec<[u8; 65]> {
    let mut out = Vec::new();
    let mut buf = header(brightness);
    let mut at = HEADER_LEN;
    let mut skip = 0;
    let mut skipped = 0;
    let mut i = 0;
    let mut ended = false;
    while i < colors.len() {
        if SKIP[skip] == i + skipped {
            skip = (skip + 1) % SKIP.len();
            skipped += 1;
            continue;
        }
        let c = colors[i];
        buf[at..at + 4].copy_from_slice(&[(i + skipped + 0x80) as u8, c.0, c.1, c.2]);
        at += COLOR_LEN;
        i += 1;
        let last = i == colors.len();
        if at + HEADER_LEN >= 65 || last {
            if last && at + COLOR_LEN < 65 {
                buf[at] = 0xFF;
                ended = true;
            }
            out.push(buf);
            buf = header(brightness);
            at = HEADER_LEN;
        }
    }
    if !ended {
        buf[at] = 0xFF;
        out.push(buf);
    }
    out
}

struct Zet;

impl Proto for Zet {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.sleep(15);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in direct_packets(colors, BRIGHTNESS) {
            io.write(&p)?;
            io.sleep(DELAY);
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let map: Vec<&[u32]> = MATRIX.iter().map(|r| &r[..]).collect();
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &map, KEYS);
    boxed(api, info, FAMILY, dev, Zet)
}

pub static DETECTORS: &[Detector] = &[det(
    "ZET Blade Optical",
    0x2EA8,
    0x2125,
    1,
    0xFF00,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // ZETBladeOpticalController::SetLEDDirect
    #[test]
    fn direct_packets_match() {
        let colors: Vec<Color> = (0..KEYS as u8).map(|i| Color(i, i, i)).collect();
        let p = direct_packets(&colors, 4);
        // 104 keys at 12 a packet: 9 packets, the last with 8 keys and the end marker
        assert_eq!(p.len(), 9);
        assert_eq!(p[0][..9], [4, 0xAE, 1, 0, 0, 0x1C, 2, 4, 0xFF]);
        // key 0 is index 0, key 1 skips index 1, keys 15.. skip 17..20
        assert_eq!(p[0][9..17], [0x80, 0, 0, 0, 0x82, 1, 1, 1]);
        assert_eq!(p[1][9..13], [0x80 + 13, 12, 12, 12]);
        assert_eq!(p[1][9 + 3 * 4..9 + 4 * 4], [0x80 + 16, 15, 15, 15]);
        assert_eq!(p[1][9 + 4 * 4..9 + 5 * 4], [0x80 + 21, 16, 16, 16]);
        assert_eq!(p[0][57..], [0; 8]);
        assert_eq!(
            p[8][9 + 7 * 4..9 + 8 * 4 + 1],
            [0x80 + 124, 103, 103, 103, 0xFF]
        );
    }

    // DeviceUpdateMode(Custom) waits 15ms, then each packet is followed by 12ms
    #[test]
    fn pacing() {
        let a = vec![Color(1, 1, 1); KEYS];
        let [first, _, _] = pushes(Zet, KEYS, vec![], &a, &[]);
        assert_eq!(first[0], Op::Sleep(15));
        assert_eq!(first.len(), 1 + 9 * 2);
        assert_eq!(first[2], Op::Sleep(12));
    }
}
