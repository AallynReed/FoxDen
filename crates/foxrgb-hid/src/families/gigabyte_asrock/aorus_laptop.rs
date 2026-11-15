//! Ported from OpenRGB Controllers/GigabyteAorusLaptopController (GPL-2.0-or-later).

use super::{Driver, Op, Plan};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

const VID: u16 = 0x0414;

pub static DETECTORS: [Detector; 2] = [
    Detector {
        name: "Gigabyte Aorus 17X Keyboard",
        vid: VID,
        pid: 0x7A3F,
        interface: Some(3),
        usage_page: Some(0xFF01),
        usage: Some(0x01),
        open,
    },
    Detector {
        name: "Gigabyte Aorus 15BKF Keyboard",
        vid: VID,
        pid: 0x7A43,
        interface: Some(3),
        usage_page: Some(0xFF01),
        usage: Some(0x01),
        open,
    },
];

const REPORT_ID: u8 = 0x08;
const BRIGHTNESS_MAX: u8 = 0x32;
const CUSTOM_MODE: u8 = 0x33;
const NA: u8 = 0xFF;
const WIDTH: usize = 19;
const HEIGHT: usize = 6;

/// aorus_laptop_default_keyboard_layout.matrix_map
const MATRIX: [[u8; WIDTH]; HEIGHT] = [
    [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    ],
    [
        19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, NA, 32, 33, 34, 35, 36,
    ],
    [
        37, NA, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
    ],
    [
        55, NA, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, NA, 67, 68, 69, 70, NA,
    ],
    [
        71, NA, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, NA, 82, 83, 84, 85, 86, 87,
    ],
    [
        88, 89, 90, 91, NA, NA, NA, 92, NA, NA, 93, 94, 95, 96, 97, 98, 99, 100, NA,
    ],
];

/// aorus_laptop_default_keyboard_layout.led_sequence_positions
const POSITIONS: [u8; 101] = [
    11, 17, 23, 29, 35, 41, 47, 53, 59, 65, 71, 77, 83, 89, 95, 101, 107, 113, 119, //
    10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 76, 82, 94, 100, 106, 112, 118, //
    9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69, 75, 81, 87, 99, 105, 111, 116, //
    8, 14, 20, 26, 32, 38, 44, 50, 56, 62, 68, 74, 92, 98, 104, 110, //
    7, 19, 25, 31, 37, 43, 49, 55, 61, 67, 73, 85, 91, 97, 103, 109, 114, //
    6, 12, 18, 24, 42, 60, 66, 72, 84, 90, 96, 102, 108,
];

/// 9-byte feature report with the trailing checksum: 0xFF minus bytes 1..8.
fn with_checksum(mut b: [u8; 9]) -> Vec<u8> {
    b[8] = b[1..8].iter().fold(0xFFu8, |s, v| s.wrapping_sub(*v));
    b.to_vec()
}

pub fn custom_mode(brightness: u8) -> Vec<u8> {
    with_checksum([
        0x00,
        REPORT_ID,
        0x00,
        CUSTOM_MODE,
        0x01,
        brightness,
        0x05,
        0x01,
        0,
    ])
}

/// SetCustom: mode, start, a feature read, the second packet, then 8 x 64 bytes of colours.
pub fn set_custom(colors: &[Color], brightness: u8) -> Vec<Op> {
    let mut ops = vec![
        Op::Feature(custom_mode(brightness)),
        Op::Feature(vec![0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6D]),
        Op::GetFeature(vec![0x00, REPORT_ID, 0, 0, 0, 0, 0, 0, 0]),
        Op::Feature(vec![0x00, 0x12, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0xE5]),
    ];
    let mut data = [0u8; 64 * 8];
    for (c, &p) in colors.iter().zip(POSITIONS.iter()) {
        let at = p as usize * 4;
        data[at + 1..at + 4].copy_from_slice(&[c.0, c.1, c.2]);
    }
    for chunk in data.chunks(64) {
        let mut b = vec![0u8];
        b.extend_from_slice(chunk);
        ops.push(Op::Write(b));
    }
    ops
}

/// Per-key control goes through Custom; Direct only sets the whole keyboard to one colour.
pub struct Keyboard;

impl Plan for Keyboard {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        set_custom(colors, BRIGHTNESS_MAX)
    }
}

fn device(id: &str, name: &str) -> Device {
    let leds = MATRIX
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .filter(|(_, k)| **k != NA)
                .map(move |(c, _)| {
                    [
                        (c as f32 + 0.5) / WIDTH as f32,
                        (r as f32 + 0.5) / HEIGHT as f32,
                    ]
                })
        })
        .collect();
    Device {
        id: id.into(),
        name: name.into(),
        kind: DeviceKind::Keyboard,
        rect: [0.06, 0.58, 0.54, 0.32],
        grid: [WIDTH as u32, HEIGHT as u32],
        leds,
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let device = device(&info.device_id("gigabyte-aorus-laptop"), d.name);
    Ok(Driver::boxed(
        "gigabyte aorus laptop",
        device,
        hid,
        Keyboard,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    // Captured packets quoted in GigabyteAorusLaptopController::SetDirect and ::SetCustom.
    #[test]
    fn checksum_matches_captured_packets() {
        assert_eq!(
            with_checksum([0x00, 0x08, 0x01, 0x00, 0x00, 0xFF, 0x32, 0x00, 0]),
            vec![0x00, 0x08, 0x01, 0x00, 0x00, 0xFF, 0x32, 0x00, 0xC5]
        );
        assert_eq!(
            custom_mode(0x2D),
            vec![0x00, 0x08, 0x00, 0x33, 0x01, 0x2D, 0x05, 0x01, 0x90]
        );
    }

    // GigabyteAorusLaptopController::SetCustom with the default layout positions.
    #[test]
    fn custom_frame_matches_set_custom() {
        let mut s = Session::new(Keyboard, 101);
        let mut colors = vec![Color::BLACK; 101];
        colors[0] = Color(1, 2, 3);
        colors[19] = Color(4, 5, 6);
        colors[100] = Color(7, 8, 9);
        let ops = s.step(&colors);
        assert_eq!(ops.len(), 12);
        assert_eq!(
            ops[0],
            Op::Feature(vec![0x00, 0x08, 0x00, 0x33, 0x01, 0x32, 0x05, 0x01, 0x8B])
        );
        assert_eq!(
            ops[1],
            Op::Feature(vec![0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6D])
        );
        assert_eq!(
            ops[2],
            Op::GetFeature(vec![0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        );
        assert_eq!(
            ops[3],
            Op::Feature(vec![0x00, 0x12, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0xE5])
        );
        let mut data = vec![0u8; 512];
        // Esc at 11*4, ` at 10*4, numpad . at 108*4
        data[45..48].copy_from_slice(&[1, 2, 3]);
        data[41..44].copy_from_slice(&[4, 5, 6]);
        data[433..436].copy_from_slice(&[7, 8, 9]);
        for i in 0..8 {
            let mut b = vec![0u8];
            b.extend_from_slice(&data[i * 64..i * 64 + 64]);
            assert_eq!(ops[4 + i], Op::Write(b), "chunk {i}");
        }
    }

    #[test]
    fn device_has_one_led_per_key() {
        let d = device("x", "y");
        assert_eq!(d.leds.len(), POSITIONS.len());
        let keys = MATRIX.iter().flatten().filter(|k| **k != NA).count();
        assert_eq!(keys, 101);
        let in_order: Vec<u8> = MATRIX
            .iter()
            .flatten()
            .copied()
            .filter(|k| *k != NA)
            .collect();
        assert_eq!(in_order, (0..101).collect::<Vec<u8>>());
    }
}
