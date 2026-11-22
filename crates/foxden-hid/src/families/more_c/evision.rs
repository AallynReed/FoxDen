// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "evision";
const SONIX_VID: u16 = 0x0C45;
const EVISION_VID: u16 = 0x320F;
const PAGE: i32 = 0xFF1C;
const LEDS: usize = 126;
const CHUNK: usize = 0x36;
/// The reply after every write is awaited; this bounds the wait.
const REPLY_MS: i32 = 1000;

const MAP: [&[u32]; 6] = [
    &[
        0, NA, 1, 2, 3, 4, NA, 5, 6, 7, 8, NA, 9, 10, 11, 12, 14, 15, 16, NA, NA, NA, NA,
    ],
    &[
        21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, NA, 32, 33, 34, NA, 35, 36, 37, 38, 39, 40, 41,
    ],
    &[
        42, NA, 43, 44, 45, 46, NA, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62,
    ],
    &[
        63, NA, 64, 65, 66, 67, NA, 68, 69, 70, 71, 72, 73, 74, 76, NA, NA, NA, NA, 80, 81, 82, NA,
    ],
    &[
        84, NA, 86, 87, 88, 89, NA, 90, NA, 91, 92, 93, 94, 95, 97, NA, NA, 99, NA, 101, 102, 103,
        104,
    ],
    &[
        105, 106, 107, NA, NA, NA, NA, 108, NA, NA, NA, NA, 109, 110, 111, 113, 119, 120, 121, 123,
        NA, 124, NA,
    ],
];

/// The checksum sums `char`, which is signed, so bytes from 0x80 count as negative.
fn packet(cmd: u8, size: u8, offset: u16, data: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x04;
    b[3] = cmd;
    b[4] = size;
    b[5..7].copy_from_slice(&offset.to_le_bytes());
    b[8..8 + data.len()].copy_from_slice(data);
    let sum = b[3..]
        .iter()
        .fold(0u16, |s, &x| s.wrapping_add(x as i8 as u16));
    b[1..3].copy_from_slice(&sum.to_le_bytes());
    b
}

fn send(io: &mut dyn Io, p: &[u8; 64]) -> Result<(), String> {
    io.write(p)?;
    let mut reply = [0u8; 64];
    let _ = io.read(&mut reply, REPLY_MS);
    Ok(())
}

struct EVision;

impl Proto for EVision {
    /// SendKeyboardModeEx for Custom at full brightness.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        send(io, &packet(0x06, 8, 0, &[0x14, 4, 0, 0, 0, 0, 0, 0]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut data = [0u8; 7 * CHUNK];
        for (i, c) in colors.iter().take(LEDS).enumerate() {
            data[i * 3..i * 3 + 3].copy_from_slice(&rgb(*c));
        }
        for (k, chunk) in data.chunks(CHUNK).enumerate() {
            send(io, &packet(0x11, CHUNK as u8, (k * CHUNK) as u16, chunk))?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MAP, LEDS);
    boxed(api, info, FAMILY, dev, EVision)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "EVision Keyboard 0C45:5004",
        SONIX_VID,
        0x5004,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 0C45:5104",
        SONIX_VID,
        0x5104,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 0C45:5204",
        SONIX_VID,
        0x5204,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 0C45:652F",
        SONIX_VID,
        0x652F,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 0C45:7672",
        SONIX_VID,
        0x7672,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 0C45:7698",
        SONIX_VID,
        0x7698,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 0C45:8520",
        SONIX_VID,
        0x8520,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 320F:5000",
        EVISION_VID,
        0x5000,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 320F:502A",
        EVISION_VID,
        0x502A,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 320F:505B",
        EVISION_VID,
        0x505B,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 320F:5078",
        EVISION_VID,
        0x5078,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 320F:5064",
        EVISION_VID,
        0x5064,
        1,
        PAGE,
        ANY,
        open,
    ),
    det(
        "EVision Keyboard 320F:5084",
        EVISION_VID,
        0x5084,
        1,
        PAGE,
        ANY,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn map_size() {
        assert!(MAP.iter().all(|r| r.len() == 23));
        let max = MAP
            .iter()
            .flat_map(|r| r.iter())
            .filter(|&&v| v != NA)
            .max();
        assert_eq!(max, Some(&124));
        assert_eq!(DETECTORS.len(), 13);
    }

    // Custom mode parameter, then seven 0x36-byte colour packets, each followed by a read
    #[test]
    fn mode_then_colour_packets() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(0x90, 2, 3);
        a[20] = Color(4, 5, 6); // bytes 60..63: packet 1 offset 6
        let mut b = a.clone();
        b[1] = Color(7, 8, 9);
        let [first, _, next] = pushes(EVision, LEDS, vec![], &a, &b);
        assert_eq!(first.len(), 2 + 7 * 2);
        assert_eq!(
            first[0],
            w(pad(&[4, 0x26, 0, 0x06, 8, 0, 0, 0, 0x14, 4], 64))
        );
        assert_eq!(first[1], Op::Read);
        // 0x11 + 0x36 - 0x70 + 2 + 3 = -36
        assert_eq!(
            first[2],
            w(pad(&[4, 0xDC, 0xFF, 0x11, 0x36, 0, 0, 0, 0x90, 2, 3], 64))
        );
        let mut p1 = pad(&[4, 0x8C, 0, 0x11, 0x36, 0x36, 0], 64);
        p1[8 + 6..8 + 9].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[4], w(p1));
        assert_eq!(first[14], w(pad(&[4, 0x8C, 0, 0x11, 0x36, 0x44, 0x01], 64)));
        assert_eq!(next.len(), 7 * 2);
    }
}
