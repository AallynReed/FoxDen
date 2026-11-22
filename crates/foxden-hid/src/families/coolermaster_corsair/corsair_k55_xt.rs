// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, matrix, report, write};
use crate::registry::{Detector, Info};

const NA: u32 = u32::MAX;
const BANK: usize = 137;
const LEDS: usize = 111;

pub static DETECTORS: &[Detector] = &[det!(
    "Corsair K55 RGB PRO XT",
    0x1B1C,
    0x1BA1,
    Some(1),
    Some(0xFF42),
    None,
    open
)];

const KEYS: [u8; LEDS] = [
    127, 128, 129, 130, 131, 132, 37, 49, 39, 53, 102, 101, 26, 96, 104, 54, 27, 16, 0, 25, 103,
    55, 28, 22, 18, 23, 56, 29, 4, 3, 2, 57, 30, 17, 5, 21, 31, 58, 32, 19, 6, 1, 40, 59, 33, 24,
    7, 60, 34, 20, 9, 13, 61, 35, 8, 10, 12, 14, 11, 50, 62, 41, 15, 47, 51, 107, 63, 42, 43, 48,
    52, 118, 64, 38, 44, 46, 106, 97, 65, 36, 105, 66, 69, 72, 76, 67, 70, 73, 78, 77, 68, 71, 74,
    75, 79, 91, 88, 85, 94, 80, 92, 89, 86, 81, 93, 90, 87, 95, 82, 83, 84,
];

const MATRIX_MAP: [[u32; 24]; 6] = [
    [
        0, 6, NA, 15, 21, 26, 31, NA, 37, 43, 47, 52, NA, 60, 66, 72, 78, 81, 85, 90, NA, NA, NA,
        NA,
    ],
    [
        1, 7, 12, 16, 22, 27, 32, 36, 38, 44, 48, 53, NA, 61, 67, 73, NA, 82, 86, 91, 94, 99, 103,
        108,
    ],
    [
        2, 8, NA, 17, 23, 28, 33, NA, 39, 45, 49, 54, 57, 62, 68, 74, 79, 83, 87, 92, 95, 100, 104,
        109,
    ],
    [
        3, 9, NA, 18, 24, 29, 34, NA, 40, 46, 50, 55, 58, 63, 69, 75, NA, NA, NA, NA, 96, 101, 105,
        NA,
    ],
    [
        4, 10, 13, 19, 25, 30, 35, NA, 41, NA, 51, 56, 59, 64, 70, 76, NA, NA, 88, NA, 97, 102,
        106, 110,
    ],
    [
        5, 11, 14, 20, NA, NA, NA, NA, 42, NA, NA, NA, NA, 65, 71, 77, 80, 84, 89, 93, 98, NA, 107,
        NA,
    ],
];

pub type Report = [u8; 65];

/// LightingControl: software mode on.
pub fn lighting_control() -> [Report; 3] {
    [
        report(&[0x08, 0x01, 0x03, 0x00, 0x02]),
        report(&[0x08, 0x02, 0x5F]),
        report(&[0x08, 0x0D, 0x00, 0x01]),
    ]
}

/// SetLEDs: red, green and blue banks of 137, sent as 53 bytes then six packets of 61.
pub fn frame(colors: &[Color]) -> [Report; 7] {
    // OpenRGB reads 8 bytes past its banks for the last packet; those are zero here.
    let mut bank = [0u8; 3 * BANK + 8];
    for (c, &k) in colors.iter().zip(KEYS.iter()) {
        bank[k as usize] = c.0;
        bank[BANK + k as usize] = c.1;
        bank[2 * BANK + k as usize] = c.2;
    }
    let mut out = [[0u8; 65]; 7];
    out[0] = report(&[0x08, 0x06, 0x00, 0x9B, 0x01]);
    out[0][12..].copy_from_slice(&bank[..53]);
    for (i, r) in out.iter_mut().enumerate().skip(1) {
        let at = 53 + (i - 1) * 61;
        r[..4].copy_from_slice(&[0x00, 0x08, 0x07, 0x00]);
        r[4..].copy_from_slice(&bank[at..at + 61]);
    }
    out
}

struct K55Xt {
    hid: HidDevice,
}

impl Driver for K55Xt {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            for r in lighting_control() {
                write(&self.hid, &r)?;
            }
        }
        for r in frame(colors) {
            write(&self.hid, &r)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let rows: Vec<&[u32]> = MATRIX_MAP.iter().map(|r| r.as_slice()).collect();
    let (leds, grid) = matrix(&rows, LEDS);
    let dev = device(
        info.device_id("corsairperipheral"),
        det.name,
        DeviceKind::Keyboard,
        leds,
        grid,
    );
    Ok(Lights::boxed(
        "corsair k55 pro xt",
        dev,
        None,
        K55Xt { hid },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CorsairK55RGBPROXTController::LightingControl
    #[test]
    fn lighting_control_bytes() {
        let [a, b, c] = lighting_control();
        assert_eq!(&a[..6], &[0x00, 0x08, 0x01, 0x03, 0x00, 0x02]);
        assert_eq!(&b[..4], &[0x00, 0x08, 0x02, 0x5F]);
        assert_eq!(&c[..5], &[0x00, 0x08, 0x0D, 0x00, 0x01]);
        assert!(a[6..].iter().chain(&b[4..]).chain(&c[5..]).all(|&x| x == 0));
    }

    // CorsairK55RGBPROXTController::SetLEDs
    #[test]
    fn frame_bytes() {
        let mut colors = vec![Color::BLACK; LEDS];
        colors[18] = Color(1, 2, 3);
        colors[0] = Color(4, 5, 6);
        let f = frame(&colors);
        assert_eq!(
            &f[0][..12],
            &[0, 0x08, 0x06, 0, 0x9B, 0x01, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(f[0][12], 1);
        // key 127 red sits at bank offset 127: packet 2 (53 + 61 = 114), byte 4 + 13
        assert_eq!(&f[2][..4], &[0, 0x08, 0x07, 0]);
        assert_eq!(f[2][4 + 13], 4);
        // green bank starts at 137: key 0 green at 137 -> packet 2, byte 4 + 23
        assert_eq!(f[2][4 + 23], 2);
        // blue of key 0 at 274 -> packet 4 (53 + 183 = 236), byte 4 + 38
        assert_eq!(f[4][4 + 38], 3);
        // blue of key 127 at 401 -> packet 6 (53 + 305 = 358), byte 4 + 43
        assert_eq!(f[6][4 + 43], 6);
        assert!(f[6][4 + 53..].iter().all(|&x| x == 0));
    }

    #[test]
    fn every_key_has_a_place() {
        let rows: Vec<&[u32]> = MATRIX_MAP.iter().map(|r| r.as_slice()).collect();
        assert_eq!(matrix(&rows, LEDS).1, [24, 6]);
    }
}
