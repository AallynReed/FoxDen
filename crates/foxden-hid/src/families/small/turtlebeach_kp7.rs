//! Ported from OpenRGB Controllers/TurtleBeachKP7Controller (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "turtlebeach-kp7";
const LEDS: usize = 33;
const REPORT: u8 = 0xEF;
const PAGE0: usize = 62;
const KEEPALIVE_MS: u64 = 50;

/// turtle_beach_kp7_led_order: device slot of each LED (indicators, keypad, wrist rest).
const ORDER: [usize; LEDS] = [
    28, 29, 30, 21, 0, 7, 14, 8, 22, 15, 1, 11, 23, 2, 9, 4, 18, 10, 24, 16, 25, 5, 17, 3, 13, 12,
    19, 27, 6, 20, 26, 31, 32,
];

const KEYPAD: [[u32; 5]; 7] = [
    [0, 1, 2, 3, NA],
    [4, 5, 6, 7, 8],
    [9, 10, 11, 12, 13],
    [14, 15, 16, 17, 18],
    [19, 20, 21, 22, 23],
    [24, NA, 25, 26, NA],
    [NA, NA, NA, 27, NA],
];

pub const ENTER_DIRECT: [u8; 5] = [14, 0x00, 0x01, 0x00, 0x00];

/// SendFrame: 99 bytes of RGB split over two pages.
pub fn frame_pages(colors: &[Color]) -> [[u8; 64]; 2] {
    let mut data = [0u8; LEDS * 3];
    for (i, c) in colors.iter().take(LEDS).enumerate() {
        let at = ORDER[i] * 3;
        data[at..at + 3].copy_from_slice(&rgb(*c));
    }
    let mut p0 = [0u8; 64];
    p0[0] = REPORT;
    p0[2..].copy_from_slice(&data[..PAGE0]);
    let mut p1 = [0u8; 64];
    p1[0] = REPORT;
    p1[1] = 0x01;
    p1[2..2 + LEDS * 3 - PAGE0].copy_from_slice(&data[PAGE0..]);
    [p0, p1]
}

struct Kp7;

impl Proto for Kp7 {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&ENTER_DIRECT)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        frame_pages(colors).iter().try_for_each(|p| io.write(p))
    }
    // OpenRGB's keepalive thread resends the last frame every 50ms
    fn keepalive_ms(&self) -> Option<u64> {
        Some(KEEPALIVE_MS)
    }
}

fn positions() -> Vec<(u32, u32)> {
    let mut at: Vec<(u32, u32)> = (0..3).map(|i| (i + 1, 0)).collect();
    for k in 0..28u32 {
        for (r, row) in KEYPAD.iter().enumerate() {
            if let Some(c) = row.iter().position(|&v| v == k) {
                at.push((c as u32, r as u32 + 1));
            }
        }
    }
    at.extend([(1, 8), (3, 8)]);
    at
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(info, FAMILY, d, DeviceKind::Keyboard, 5, 9, &positions());
    boxed(api, info, FAMILY, dev, Kp7)
}

pub static DETECTORS: &[Detector] = &[det(
    "Turtle Beach Command Series KP7",
    0x10F5,
    0x503A,
    4,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout() {
        assert_eq!(positions().len(), LEDS);
    }

    // EnterDirectMode from the constructor, then SendDirect -> SendFrame
    #[test]
    fn enter_direct_then_pages() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3); // indicator 1, slot 28
        a[4] = Color(4, 5, 6); // keypad LED 1, slot 0
        a[32] = Color(7, 8, 9); // wrist rest 2, slot 32
        let [first, _, _] = pushes(Kp7, LEDS, vec![], &a, &[]);
        assert_eq!(first[0], Op::F(vec![14, 0, 1, 0, 0]));
        let p0 = pad(&[0xEF, 0, 4, 5, 6], 64);
        let mut p1 = pad(&[0xEF, 1], 64);
        p1[2 + 84 - 62..2 + 87 - 62].copy_from_slice(&[1, 2, 3]);
        p1[2 + 96 - 62..2 + 99 - 62].copy_from_slice(&[7, 8, 9]);
        assert_eq!(first[1..], [w(p0), w(p1)]);
    }
}
