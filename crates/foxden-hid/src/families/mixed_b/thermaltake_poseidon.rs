// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{NA, Op, Protocol, at, device, matrix, on_ip, open_with};
use crate::registry::{Detector, Info};

const LEDS: usize = 104;

/// Report offset of each LED.
const KEYS: [usize; LEDS] = [
    0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x15, 0x16, 0x17, 0x18,
    0x19, 0x1A, 0x1B, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x26, 0x27, 0x28, 0x29, 0x2A,
    0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B,
    0x3C, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4E,
    0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E,
    0x5F, 0x60, 0x61, 0x62, 0x63, 0x64, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6C, 0x6D, 0x6F, 0x70, 0x72,
    0x73, 0x75, 0x76, 0x77, 0x78, 0x7C, 0x80, 0x81,
];

#[rustfmt::skip]
const MAP: [u32; 6 * 23] = [
    0, NA, 8, 15, 22, 29, NA, 37, 44, 51, 58, NA, 65, 73, 81, 88, 94, 100, 102, NA, NA, NA, NA,
    1, 9, 16, 23, 30, 38, 45, 52, 59, 66, 74, NA, 82, 89, 103, NA, 7, 21, 36, 50, 64, 80, 93,
    2, NA, 10, 17, 24, 31, NA, 39, 46, 53, 60, 67, 75, 83, 90, 95, 14, 28, 43, 57, 72, 87, 86,
    3, NA, 11, 18, 25, 32, NA, 40, 47, 54, 61, 68, 76, 84, 96, NA, NA, NA, NA, 35, 99, 63, NA,
    4, NA, 26, 33, 41, 48, NA, 55, NA, 62, 69, 77, 85, 91, 101, NA, NA, 27, NA, 42, 49, 71, 98,
    5, 12, 19, NA, NA, NA, NA, 34, NA, NA, NA, NA, 70, 78, 92, 97, 6, 13, 20, 56, NA, 79, NA,
];

/// `PoseidonZRGBController::SendControl` for profile 1 at full brightness.
fn control(direction: u8, mode: u8, speed: u8) -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[0x00] = 0x07;
    b[0x01] = 0x02;
    b[0x02] = 0x01;
    b[0x08] = 0x01;
    b[0x0A] = direction;
    b[0x0C] = mode;
    b[0x0D] = 0x04;
    b[0x10] = 0x08;
    b[0x12] = speed;
    b[0x13] = 0x50;
    b
}

/// `PoseidonZRGBController::SetLEDsDirect`: red and green in one report, blue in another.
fn direct(colors: &[Color]) -> (Vec<u8>, Vec<u8>) {
    let mut rg = vec![0u8; 264];
    let mut b = vec![0u8; 264];
    rg[..4].copy_from_slice(&[0x07, 0x0E, 0x01, 0x01]);
    b[..4].copy_from_slice(&[0x07, 0x0E, 0x01, 0x02]);
    for (i, &key) in KEYS.iter().enumerate() {
        let c = at(colors, i);
        rg[key] = c.0;
        rg[key + 128] = c.1;
        b[key] = c.2;
    }
    (rg, b)
}

struct Poseidon;

impl Protocol for Poseidon {
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        vec![Op::Feature(control(0, 0x00, 0)), Op::Sleep(200)]
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let (rg, b) = direct(colors);
        vec![Op::Feature(rg), Op::Sleep(5), Op::Feature(b)]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "thermaltake poseidon", |_| {
        let leds = matrix(&MAP, 23, LEDS, |_| [0.5, 0.5]);
        let dev = device(
            info.device_id("thermaltake-poseidon"),
            d.name,
            DeviceKind::Keyboard,
            [23, 6],
            leds,
        );
        Ok((dev, Poseidon))
    })
}

pub(super) static DETECTORS: [Detector; 1] = [on_ip(
    "Thermaltake Poseidon Z RGB",
    0x264A,
    0x3006,
    1,
    0xFF01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::super::test_util::buf;
    use super::*;

    // RGBController_PoseidonZRGB::DeviceUpdateMode with Direct (static, direction 0, speed 0)
    #[test]
    fn setup_selects_static_profile_one() {
        let mut want = buf(264, &[0x07, 0x02, 0x01]);
        want[0x08] = 0x01;
        want[0x0D] = 0x04;
        want[0x10] = 0x08;
        want[0x13] = 0x50;
        assert_eq!(Poseidon.setup(&[]), vec![Op::Feature(want), Op::Sleep(200)]);
    }

    // PoseidonZRGBController::SetLEDsDirect
    #[test]
    fn frame_splits_channels() {
        let mut colors = vec![Color::BLACK; LEDS];
        colors[0] = Color(0x11, 0x22, 0x33);
        colors[103] = Color(0x44, 0x55, 0x66);
        let ops = Poseidon.frame(&colors);
        let mut rg = buf(264, &[0x07, 0x0E, 0x01, 0x01]);
        rg[0x08] = 0x11;
        rg[0x08 + 128] = 0x22;
        rg[0x81] = 0x44;
        rg[0x81 + 128] = 0x55;
        let mut b = buf(264, &[0x07, 0x0E, 0x01, 0x02]);
        b[0x08] = 0x33;
        b[0x81] = 0x66;
        assert_eq!(ops, vec![Op::Feature(rg), Op::Sleep(5), Op::Feature(b)]);
    }

    #[test]
    fn every_key_is_on_the_map() {
        for led in 0..LEDS as u32 {
            assert!(MAP.contains(&led), "{led}");
        }
    }
}
