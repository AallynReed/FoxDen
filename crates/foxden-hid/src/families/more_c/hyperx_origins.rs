//! Ported from OpenRGB Controllers/HyperXKeyboardController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-origins";
const LEDS: usize = 107;
const SKIP: [usize; 19] = [
    23, 29, 41, 47, 59, 70, 71, 87, 88, 93, 99, 100, 102, 108, 113, 114, 120, 123, 124,
];

const MATRIX: [&[u32]; 6] = [
    &[
        0, NA, 12, 18, 23, 28, NA, 34, 39, 44, 50, NA, 55, 61, 65, 71, 77, 81, 86, NA, NA, NA, NA,
    ],
    &[
        1, 7, 13, 19, 24, 29, 35, 40, 45, 51, 56, NA, 62, 66, 71, 72, 78, 82, 87, 90, 95, 99, 104,
    ],
    &[
        2, NA, 8, 14, 20, 25, NA, 30, 36, 41, 46, 52, 57, 63, 67, 73, 79, 83, 88, 91, 96, 100, 105,
    ],
    &[
        3, NA, 9, 15, 21, 26, NA, 31, 37, 42, 47, 53, 58, 64, 68, 74, NA, NA, NA, 92, 97, 101, NA,
    ],
    &[
        4, 6, 10, 16, 22, 27, NA, 32, NA, 38, 43, 48, 54, 59, 69, 75, NA, 84, NA, 93, 98, 102, 106,
    ],
    &[
        5, 11, 17, NA, NA, NA, NA, 33, NA, NA, NA, NA, 49, 60, 70, 76, 80, 85, 89, 94, NA, 103, NA,
    ],
];

/// Colours with a black slot inserted at each of `skip`, in order.
pub fn with_gaps(colors: &[Color], skip: &[usize]) -> Vec<Color> {
    let mut v = colors.to_vec();
    for &i in skip {
        v.insert(i.min(v.len()), Color::BLACK);
    }
    v
}

/// SendDirectColorPacket: up to 16 colours as 0x81, R, G, B.
fn color_packet(colors: &[Color]) -> [u8; 65] {
    let mut b = [0u8; 65];
    for (i, c) in colors.iter().take(16).enumerate() {
        b[i * 4 + 1] = 0x81;
        b[i * 4 + 2..i * 4 + 5].copy_from_slice(&rgb(*c));
    }
    b
}

/// SetLEDsDirect: init feature report, then `count` colour reports, one black colour once they run out.
pub fn send_direct(
    io: &mut dyn Io,
    init_9: u8,
    slots: &[Color],
    count: usize,
) -> Result<(), String> {
    let mut init = [0u8; 65];
    init[1] = 0x04;
    init[2] = 0xF2;
    init[9] = init_9;
    io.feature(&init)?;
    let mut chunks = slots.chunks(16);
    for _ in 0..count {
        let p = color_packet(chunks.next().unwrap_or(&[Color::BLACK]));
        io.feature(&p)?;
    }
    Ok(())
}

struct Origins;

impl Proto for Origins {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send_direct(io, 0x09, &with_gaps(colors, &SKIP), 9)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MATRIX, LEDS);
    boxed(api, info, FAMILY, dev, Origins)
}

pub static DETECTORS: &[Detector] = &[
    det("HyperX Alloy Origins", 0x0951, 0x16E5, 3, ANY, ANY, open),
    det(
        "HyperX Alloy Origins (HP)",
        0x03F0,
        0x0591,
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
        assert_eq!(MATRIX.iter().map(|r| r.len()).sum::<usize>(), 6 * 23);
        let mut seen: Vec<u32> = MATRIX.iter().flat_map(|r| r.iter().copied()).collect();
        seen.retain(|&v| v != NA);
        seen.sort();
        seen.dedup();
        assert_eq!(seen, (0..LEDS as u32).collect::<Vec<_>>());
    }

    #[test]
    fn init_then_packets() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[23] = Color(4, 5, 6); // slot 24 after the blank at 23: packet 1, colour 8
        a[106] = Color(7, 8, 9); // slot 125: packet 7, colour 13
        let [first, _, _] = pushes(Origins, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 10);
        assert_eq!(
            first[0],
            Op::F(pad(&[0, 0x04, 0xF2, 0, 0, 0, 0, 0, 0, 0x09], 65))
        );
        let mut p0 = vec![0u8; 65];
        for i in 0..16 {
            p0[i * 4 + 1] = 0x81;
        }
        p0[2..5].copy_from_slice(&[1, 2, 3]);
        assert_eq!(first[1], Op::F(p0));
        let mut p1 = vec![0u8; 65];
        for i in 0..16 {
            p1[i * 4 + 1] = 0x81;
        }
        p1[8 * 4 + 2..8 * 4 + 5].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[2], Op::F(p1));
        let mut p7 = vec![0u8; 65];
        for i in 0..14 {
            p7[i * 4 + 1] = 0x81;
        }
        p7[13 * 4 + 2..13 * 4 + 5].copy_from_slice(&[7, 8, 9]);
        assert_eq!(first[8], Op::F(p7));
        assert_eq!(first[9], Op::F(pad(&[0, 0x81], 65)));
    }
}
