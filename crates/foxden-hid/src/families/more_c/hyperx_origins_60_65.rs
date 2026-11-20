//! Ported from OpenRGB Controllers/HyperXKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::hyperx_origins::send_direct;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-origins-60-65";
const ORIGINS_65_HP_PID: u16 = 0x038F;

const MAP_60: [&[u32]; 5] = [
    &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15],
    &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, NA, 43],
    &[44, NA, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, NA, 57],
    &[58, 59, 60, 61, NA, NA, 62, NA, NA, 63, 64, 65, 66, 70],
];

const MAP_65: [&[u32]; 5] = [
    &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 69],
    &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 70],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, NA, 43, 71],
    &[44, NA, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 57, 73, 72],
    &[58, 59, 60, 61, NA, NA, 62, NA, NA, 63, 64, 68, 74, 75, 76],
];

struct Origins60;

impl Proto for Origins60 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send_direct(io, 0x05, colors, 5)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (map, n): (&[&[u32]], usize) = if info.pid == ORIGINS_65_HP_PID {
        (&MAP_65, 77)
    } else {
        (&MAP_60, 71)
    };
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, map, n);
    boxed(api, info, FAMILY, dev, Origins60)
}

pub static DETECTORS: &[Detector] = &[
    det("HyperX Alloy Origins 60", 0x0951, 0x1734, 3, ANY, ANY, open),
    det(
        "HyperX Alloy Origins 60 (HP)",
        0x03F0,
        0x0C8E,
        3,
        ANY,
        ANY,
        open,
    ),
    det(
        "HyperX Alloy Origins 65 (HP)",
        0x03F0,
        ORIGINS_65_HP_PID,
        3,
        ANY,
        ANY,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn maps_match_openrgb() {
        assert_eq!(MAP_60.iter().map(|r| r.len()).sum::<usize>(), 5 * 14);
        assert_eq!(MAP_65.iter().map(|r| r.len()).sum::<usize>(), 5 * 15);
        let max = |m: &[&[u32]]| {
            m.iter()
                .flat_map(|r| r.iter())
                .filter(|&&v| v != NA)
                .max()
                .copied()
        };
        assert_eq!(max(&MAP_60), Some(70));
        assert_eq!(max(&MAP_65), Some(76));
    }

    #[test]
    fn init_then_packets() {
        let mut a = vec![Color::BLACK; 71];
        a[0] = Color(1, 2, 3);
        a[70] = Color(4, 5, 6); // packet 4, colour 6
        let [first, _, _] = pushes(Origins60, 71, vec![], &a, &[]);
        assert_eq!(first.len(), 6);
        assert_eq!(
            first[0],
            Op::F(pad(&[0, 0x04, 0xF2, 0, 0, 0, 0, 0, 0, 0x05], 65))
        );
        let mut p0 = vec![0u8; 65];
        for i in 0..16 {
            p0[i * 4 + 1] = 0x81;
        }
        p0[2..5].copy_from_slice(&[1, 2, 3]);
        assert_eq!(first[1], Op::F(p0));
        let mut p4 = vec![0u8; 65];
        for i in 0..7 {
            p4[i * 4 + 1] = 0x81;
        }
        p4[6 * 4 + 2..6 * 4 + 5].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[5], Op::F(p4));
    }
}
