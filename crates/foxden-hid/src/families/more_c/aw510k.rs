//! Ported from OpenRGB Controllers/AlienwareKeyboardController (GPL-2.0-or-later).

use super::aw410k::{commit, feature, send_keys};
use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "alienware-aw510k";

const MAP: [&[u32]; 7] = [
    &[
        0, NA, 1, 2, 3, 4, NA, 5, 6, 7, 8, 9, 10, 11, 12, NA, 13, 14, 15, NA, 16, NA, NA, NA,
    ],
    &[
        17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, NA, NA, 31, 32, 33, NA, 34, 35, 36,
        37,
    ],
    &[
        38, NA, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, NA, 52, 53, 54, NA, 55, 56, 57,
        74,
    ],
    &[
        58, NA, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, NA, 70, NA, NA, NA, NA, NA, 71, 72, 73,
        NA,
    ],
    &[
        75, NA, NA, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, NA, NA, NA, 87, NA, NA, 88, 89, 90,
        104,
    ],
    &[
        91, NA, 92, 93, NA, 94, NA, NA, NA, NA, NA, 95, 96, 97, 98, NA, 99, 100, 101, NA, 102, NA,
        103, NA,
    ],
    &[
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 105, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA,
        NA, NA,
    ],
];

/// The key index each LED is addressed by; the last is the logo.
const KEYS: [u8; 106] = [
    0xB0, 0x98, 0x90, 0x88, 0x80, 0x70, 0x68, 0x60, 0x58, 0x50, 0x48, 0x40, 0x38, 0x30, 0x28, 0x20,
    0x18, 0xB1, 0xA1, 0x99, 0x91, 0x89, 0x81, 0x79, 0x71, 0x69, 0x61, 0x59, 0x51, 0x49, 0x39, 0x31,
    0x29, 0x21, 0x19, 0x11, 0x09, 0x01, 0xB2, 0xA2, 0x9A, 0x92, 0x8A, 0x82, 0x7A, 0x72, 0x6A, 0x62,
    0x5A, 0x52, 0x4A, 0x42, 0x32, 0x2A, 0x22, 0x1A, 0x12, 0x0A, 0xB3, 0xA3, 0x9B, 0x93, 0x8B, 0x83,
    0x7B, 0x73, 0x6B, 0x63, 0x5B, 0x53, 0x43, 0x1B, 0x13, 0x0B, 0x03, 0xB4, 0xA4, 0x9C, 0x94, 0x8C,
    0x84, 0x7C, 0x74, 0x6C, 0x64, 0x5C, 0x4C, 0x2C, 0x1C, 0x14, 0x0C, 0xB5, 0xAD, 0xA5, 0x85, 0x65,
    0x5D, 0x55, 0x4D, 0x35, 0x2D, 0x25, 0x1D, 0x0D, 0x05, 0x07,
];

/// Only keys whose colour changed since the last frame are sent.
struct Aw510k {
    current: Vec<Color>,
}

impl Proto for Aw510k {
    /// The constructor's commit, then Direct falls into SetMode's default case.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        commit(io)?;
        feature(io, [0x05, 0x01, 0x51, 0x00])?;
        let mut mode = [0u8; 65];
        mode[1..16].copy_from_slice(&[
            0x05, 0x01, 0x01, 0, 0, 0, 0, 0, 0, 0, 0x0A, 0, 0x01, 0x01, 0,
        ]);
        io.write(&mode)?;
        commit(io)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let keys = KEYS
            .iter()
            .zip(colors.iter().zip(&self.current))
            .filter(|(_, (new, old))| new != old)
            .map(|(&k, (&c, _))| (k, c))
            .collect();
        send_keys(io, keys)?;
        self.current = colors.to_vec();
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MAP, KEYS.len());
    let proto = Aw510k {
        current: vec![Color::BLACK; KEYS.len()],
    };
    boxed(api, info, FAMILY, dev, proto)
}

pub static DETECTORS: &[Detector] = &[det(
    "Alienware AW510K",
    0x04F2,
    0x1830,
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
        assert_eq!(max, Some(&105));
        assert_eq!(KEYS.len(), 106);
    }

    #[test]
    fn start_then_changed_keys_only() {
        let proto = || Aw510k {
            current: vec![Color::BLACK; 106],
        };
        let mut a = vec![Color::BLACK; 106];
        a[105] = Color(1, 2, 3);
        let mut b = a.clone();
        b[0] = Color(4, 5, 6);
        let [first, _, next] = pushes(proto(), 106, vec![], &a, &b);
        let mut commit = vec![0u8; 65];
        commit[1..15].copy_from_slice(&[5, 1, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x0A, 1, 2, 1]);
        assert_eq!(first[0], w(commit.clone()));
        assert_eq!(first[2], Op::F(pad(&[0, 5, 1, 0x51, 0], 65)));
        let mut mode = vec![0u8; 65];
        mode[1..16].copy_from_slice(&[5, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0x0A, 0, 1, 1, 0]);
        assert_eq!(first[4], w(mode));
        assert_eq!(first[5], w(commit));
        // start (7 ops), init/feature/commit (6), count feature (2), one packet
        assert_eq!(first.len(), 7 + 6 + 2 + 1);
        assert_eq!(first[13], Op::F(pad(&[0, 0x0E, 1, 0, 1], 65)));
        let mut p = vec![0u8; 65];
        p[1..5].copy_from_slice(&[0x0E, 1, 0, 1]);
        for (at, idx, c) in [
            (5, 0x07, [1, 2, 3]),
            (0x14, 0, [0, 0, 0]),
            (0x23, 0, [0, 0, 0]),
            (0x32, 0, [0, 0, 0]),
        ] {
            p[at..at + 4].copy_from_slice(&[idx, 0x81, 0, 0xA5]);
            p[at + 5..at + 9].copy_from_slice(&[0x0A, c[0], c[1], c[2]]);
            p[at + 13] = 1;
        }
        assert_eq!(first[15], w(p));
        assert_eq!(next[6], Op::F(pad(&[0, 0x0E, 1, 0, 1], 65)));
        let Op::W(q) = &next[8] else { panic!() };
        assert_eq!(q[5..14], [0xB0, 0x81, 0, 0xA5, 0, 0x0A, 4, 5, 6]);
        assert_eq!(next.len(), 9);
    }
}
