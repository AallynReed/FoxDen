// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "alienware-aw410k";

const MAP: [&[u32]; 6] = [
    &[
        0, NA, 1, 2, 3, 4, NA, 5, 6, 7, 8, 9, 10, 11, 12, NA, 13, 14, 15, NA, NA, 16, 17, 18,
    ],
    &[
        19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, NA, NA, 33, 34, 35, NA, 36, 37, 38,
        39,
    ],
    &[
        40, NA, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, NA, 54, 55, 56, NA, 57, 58, 59,
        60,
    ],
    &[
        61, NA, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, NA, 73, NA, NA, NA, NA, NA, 74, 75, 76,
        NA,
    ],
    &[
        77, NA, NA, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, NA, NA, NA, 89, NA, NA, 90, 91, 92,
        93,
    ],
    &[
        94, NA, 95, 96, NA, NA, NA, 97, NA, NA, NA, 98, 99, 100, 101, NA, 102, 103, 104, NA, 105,
        NA, 106, NA,
    ],
];

/// The key index each LED is addressed by.
const KEYS: [u8; 107] = [
    0xB0, 0x98, 0x90, 0x88, 0x80, 0x70, 0x68, 0x60, 0x58, 0x50, 0x48, 0x40, 0x38, 0x30, 0x28, 0x20,
    0x18, 0x10, 0x08, 0xB1, 0xA1, 0x99, 0x91, 0x89, 0x81, 0x79, 0x71, 0x69, 0x61, 0x59, 0x51, 0x49,
    0x39, 0x31, 0x29, 0x21, 0x19, 0x11, 0x09, 0x01, 0xB2, 0xA2, 0x9A, 0x92, 0x8A, 0x82, 0x7A, 0x72,
    0x6A, 0x62, 0x5A, 0x52, 0x4A, 0x42, 0x32, 0x2A, 0x22, 0x1A, 0x12, 0x0A, 0x03, 0xB3, 0xA3, 0x9B,
    0x93, 0x8B, 0x83, 0x7B, 0x73, 0x6B, 0x63, 0x5B, 0x53, 0x43, 0x1B, 0x13, 0x0B, 0xB4, 0xA4, 0x9C,
    0x94, 0x8C, 0x84, 0x7C, 0x74, 0x6C, 0x64, 0x5C, 0x4C, 0x2C, 0x1C, 0x14, 0x0C, 0x05, 0xB5, 0xAD,
    0xA5, 0x85, 0x65, 0x5D, 0x55, 0x4D, 0x35, 0x2D, 0x25, 0x1D, 0x0D,
];

fn packet(at: &[(usize, u8)]) -> [u8; 65] {
    let mut b = [0u8; 65];
    for &(i, v) in at {
        b[i] = v;
    }
    b
}

/// SendCommit, which the constructor also sends.
pub fn commit(io: &mut dyn Io) -> Result<(), String> {
    io.write(&packet(&[
        (0x01, 0x05),
        (0x02, 0x01),
        (0x0A, 0x10),
        (0x0B, 0x0A),
        (0x0C, 0x01),
        (0x0D, 0x02),
        (0x0E, 0x01),
    ]))?;
    io.sleep(20);
    Ok(())
}

/// SendfeatureReport.
pub fn feature(io: &mut dyn Io, head: [u8; 4]) -> Result<(), String> {
    let mut b = [0u8; 65];
    b[1..5].copy_from_slice(&head);
    io.feature(&b)?;
    io.sleep(10);
    Ok(())
}

/// DeviceUpdateLEDs: initialize, commit, then the keys four to a packet.
pub fn send_keys(io: &mut dyn Io, mut keys: Vec<(u8, Color)>) -> Result<(), String> {
    io.write(&packet(&[
        (0x01, 0x0E),
        (0x02, 0x01),
        (0x04, 0x01),
        (0x05, 0xAD),
        (0x06, 0x80),
        (0x07, 0x10),
        (0x08, 0xA5),
        (0x0A, 0x0A),
        (0x12, 0x01),
    ]))?;
    io.sleep(2);
    feature(io, [0x05, 0x01, 0x51, 0x00])?;
    commit(io)?;
    if keys.is_empty() {
        return Ok(());
    }
    feature(io, [0x0E, keys.len() as u8, 0x00, 0x01])?;
    keys.resize(keys.len().div_ceil(4) * 4, (0, Color::BLACK));
    for (f, four) in keys.chunks(4).enumerate() {
        let mut b = packet(&[(0x01, 0x0E), (0x02, 0x01), (0x04, f as u8 + 1)]);
        for (j, &(idx, c)) in four.iter().enumerate() {
            let at = 0x05 + 15 * j;
            b[at] = idx;
            b[at + 1] = 0x81;
            b[at + 3] = 0xA5;
            b[at + 5] = 0x0A;
            b[at + 6..at + 9].copy_from_slice(&rgb(c));
            b[at + 13] = 0x01;
        }
        io.write(&b)?;
    }
    Ok(())
}

struct Aw410k;

impl Proto for Aw410k {
    /// The constructor's commit, then Direct's mode report.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        commit(io)?;
        feature(io, [0x05, 0x01, 0x51, 0x00])
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send_keys(
            io,
            KEYS.iter().copied().zip(colors.iter().copied()).collect(),
        )
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MAP, KEYS.len());
    boxed(api, info, FAMILY, dev, Aw410k)
}

pub static DETECTORS: &[Detector] = &[det(
    "Alienware AW410K",
    0x04F2,
    0x1968,
    0x02,
    0xFF00,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn tables_match_openrgb() {
        assert!(MAP.iter().all(|r| r.len() == 24));
        let max = MAP
            .iter()
            .flat_map(|r| r.iter())
            .filter(|&&v| v != NA)
            .max();
        assert_eq!(max, Some(&106));
        assert_eq!(KEYS.len(), 107);
    }

    #[test]
    fn start_then_direct_packets() {
        let mut a = vec![Color::BLACK; 107];
        a[0] = Color(1, 2, 3);
        a[5] = Color(4, 5, 6); // second packet, second key
        let [first, _, _] = pushes(Aw410k, 107, vec![], &a, &[]);
        let mut commit = vec![0u8; 65];
        commit[1..15].copy_from_slice(&[5, 1, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x0A, 1, 2, 1]);
        assert_eq!(first[0], w(commit.clone()));
        assert_eq!(first[1], Op::Sleep(20));
        assert_eq!(first[2], Op::F(pad(&[0, 5, 1, 0x51, 0], 65)));
        assert_eq!(first[3], Op::Sleep(10));
        let mut init = vec![0u8; 65];
        init[1..19].copy_from_slice(&[
            0x0E, 1, 0, 1, 0xAD, 0x80, 0x10, 0xA5, 0, 0x0A, 0, 0, 0, 0, 0, 0, 0, 1,
        ]);
        assert_eq!(first[4], w(init));
        assert_eq!(first[5], Op::Sleep(2));
        assert_eq!(first[6], Op::F(pad(&[0, 5, 1, 0x51, 0], 65)));
        assert_eq!(first[8], w(commit));
        assert_eq!(first[10], Op::F(pad(&[0, 0x0E, 107, 0, 1], 65)));
        assert_eq!(first[11], Op::Sleep(10));
        assert_eq!(first.len(), 12 + 27);
        let mut p0 = vec![0u8; 65];
        p0[1..5].copy_from_slice(&[0x0E, 1, 0, 1]);
        for (at, idx, c) in [
            (5, 0xB0, [1, 2, 3]),
            (0x14, 0x98, [0, 0, 0]),
            (0x23, 0x90, [0, 0, 0]),
            (0x32, 0x88, [0, 0, 0]),
        ] {
            p0[at..at + 4].copy_from_slice(&[idx, 0x81, 0, 0xA5]);
            p0[at + 5..at + 9].copy_from_slice(&[0x0A, c[0], c[1], c[2]]);
            p0[at + 13] = 1;
        }
        assert_eq!(first[12], w(p0));
        let Op::W(p1) = &first[13] else { panic!() };
        assert_eq!(p1[4], 2);
        assert_eq!(p1[0x14..0x1D], [0x70, 0x81, 0, 0xA5, 0, 0x0A, 4, 5, 6]);
        // 107 keys padded to 108: the last packet holds 0x25, 0x1D, 0x0D and one empty slot
        let Op::W(last) = &first[38] else { panic!() };
        assert_eq!(
            (last[4], last[5], last[0x14], last[0x23], last[0x32]),
            (27, 0x25, 0x1D, 0x0D, 0)
        );
        assert_eq!(last[0x33], 0x81);
    }
}
