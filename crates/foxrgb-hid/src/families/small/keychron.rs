//! Ported from OpenRGB Controllers/KeychronKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::mintaka::Pages;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "keychron";

/// k3_keychron: LED numbers in column-major order.
const MATRIX: [[u32; 16]; 6] = [
    [0, 6, 9, 15, 20, 25, 30, 36, 41, 46, 51, 57, 63, 69, 73, 78],
    [1, 7, 10, 16, 21, 26, 31, 37, 42, 47, 52, 58, 64, 70, NA, 79],
    [
        2, NA, 11, 17, 22, 27, 32, 38, 43, 48, 53, 59, 65, 71, 74, 80,
    ],
    [
        3, NA, 12, 18, 23, 28, 33, 39, 44, 49, 54, 60, 66, NA, 75, 81,
    ],
    [
        4, NA, 13, 19, 24, 29, 34, 40, 45, 50, 55, 61, 67, NA, 76, 82,
    ],
    [5, 8, 14, NA, NA, NA, 35, NA, NA, NA, 56, 62, 68, 72, 77, 83],
];

const POSITIONS: [u32; 84] = [
    143, 18, 34, 50, 66, 81, 2, 19, 82, 3, 20, 35, 51, 67, 83, 4, 21, 36, 52, 68, 5, 22, 37, 53,
    69, 6, 23, 38, 54, 70, 7, 24, 39, 55, 71, 84, 8, 25, 40, 56, 72, 9, 26, 41, 57, 73, 10, 27, 42,
    58, 74, 11, 28, 43, 59, 75, 85, 12, 29, 44, 60, 76, 86, 13, 30, 45, 61, 77, 87, 14, 31, 46, 88,
    15, 47, 63, 78, 89, 16, 32, 48, 64, 79, 90,
];

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    // the Valkyrie VK99 shares this vid/pid; its 0xFF13 collection marks it as not a Keychron
    let valkyrie = crate::device_list(api).iter().any(|x| {
        x.vendor_id() == info.vid
            && x.product_id() == info.pid
            && x.serial_number().unwrap_or_default() == info.serial
            && x.usage_page() == 0xFF13
    });
    if valkyrie {
        return Err("Valkyrie VK99, not a Keychron".into());
    }
    let map: Vec<&[u32]> = MATRIX.iter().map(|r| &r[..]).collect();
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &map, POSITIONS.len());
    boxed(api, info, FAMILY, dev, Pages(&POSITIONS))
}

pub static DETECTORS: &[Detector] = &[det(
    "Keychron Gaming Keyboard 1",
    0x05AC,
    0x024F,
    0,
    0x0001,
    0x06,
    open,
)];

#[cfg(test)]
mod tests {
    use super::super::mintaka::custom_pages;
    use super::*;
    use foxrgb_core::Color;

    #[test]
    fn matrix_numbers_every_led_once() {
        let mut seen: Vec<u32> = MATRIX
            .iter()
            .flatten()
            .copied()
            .filter(|&v| v != NA)
            .collect();
        seen.sort();
        assert_eq!(seen, (0..84).collect::<Vec<u32>>());
    }

    // KeychronKeyboardController::SetMode colour pages use led_sequence_positions
    #[test]
    fn colour_slots() {
        let mut a = vec![Color::BLACK; 84];
        a[0] = Color(1, 2, 3); // slot 143, the last slot of the buffer
        let pages = custom_pages(&POSITIONS, &a);
        assert_eq!(pages.len(), 5 + 3 + 9 + 1);
        assert_eq!(pages[16][60..], [0x80, 1, 2, 3]);
    }
}
