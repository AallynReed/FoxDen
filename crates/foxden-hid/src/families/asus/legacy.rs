// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-legacy";
const LEGACY_VID: u16 = 0x195D;
const ASUS_VID: u16 = 0x0B05;
const PROFILE: u8 = 1;
const CERBERUS_CUSTOM: u8 = 7;
const CERBERUS_BRIGHTNESS: u8 = 4;
const SAGARIS_STATIC: u8 = 1;
const SAGARIS_BRIGHTNESS: u8 = 3;

const CERBERUS_MAP: [&[u32]; 6] = [
    &[
        0, NA, 9, 15, 20, 25, NA, 35, 40, 45, 50, 56, 62, 67, 72, NA, 75, 79, 84, NA, NA, NA, NA,
        NA,
    ],
    &[
        1, 6, 10, 16, 21, 26, 30, 36, 41, 46, 51, 57, 63, 68, NA, NA, 76, 80, 85, NA, 88, 93, 97,
        102,
    ],
    &[
        2, NA, 11, 17, 22, 27, 31, 37, 42, 47, 52, 58, 64, 69, NA, NA, 77, 81, 86, NA, 89, 94, 98,
        103,
    ],
    &[
        3, NA, 12, 18, 23, 28, 32, 38, 43, 48, 53, 59, 65, 70, 73, NA, NA, NA, NA, NA, 90, 95, 99,
        NA,
    ],
    &[
        4, 7, 13, 19, 24, 29, 33, 39, 44, 49, 54, 60, NA, 71, NA, NA, NA, 82, NA, NA, 91, 96, 100,
        104,
    ],
    &[
        5, 8, 14, NA, NA, NA, 34, NA, NA, NA, 55, 61, 66, NA, 74, NA, 78, 83, 87, NA, 92, NA, 101,
        NA,
    ],
];

const CERBERUS_KEYS: [u8; 105] = [
    0x0B, 0x0E, 0x09, 0x11, 0x79, 0x06, 0x0F, 0x13, 0x7C, 0x16, 0x17, 0x08, 0x0A, 0x0C, 0x4B, 0x1E,
    0x1F, 0x10, 0x12, 0x14, 0x19, 0x27, 0x18, 0x1A, 0x1C, 0x1B, 0x26, 0x20, 0x22, 0x24, 0x2E, 0x21,
    0x23, 0x25, 0x5B, 0x07, 0x2F, 0x29, 0x2B, 0x2D, 0x33, 0x37, 0x28, 0x2A, 0x2C, 0x39, 0x3F, 0x30,
    0x32, 0x34, 0x3E, 0x47, 0x38, 0x3A, 0x3C, 0x4D, 0x56, 0x46, 0x40, 0x42, 0x45, 0x7D, 0x57, 0x36,
    0x41, 0x43, 0x3D, 0x53, 0x51, 0x31, 0x44, 0x7A, 0x55, 0x54, 0x04, 0x4F, 0x66, 0x5E, 0x75, 0x48,
    0x76, 0x77, 0x73, 0x5D, 0x00, 0x6E, 0x6F, 0x65, 0x5C, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x60, 0x61,
    0x62, 0x6C, 0x68, 0x69, 0x6A, 0x6B, 0x6D, 0x70, 0x72,
];

fn cerberus_report(head: &[u8]) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..head.len()].copy_from_slice(head);
    b
}

/// The Cerberus has no Direct; its per-LED Custom mode is driven instead.
struct Cerberus;

impl Proto for Cerberus {
    /// DeviceUpdateMode(Custom): SetPerLEDMode, then SetMode(CUSTOM, black, direction 1).
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        // SetPerLEDMode(STATIC) fills 0x00, not 0xFF
        for i in 0..4 {
            io.feature(&cerberus_report(&[0x07, 0x0E, PROFILE, i]))?;
        }
        io.feature(&cerberus_report(&[
            0x07,
            0x0A,
            PROFILE,
            CERBERUS_CUSTOM,
            0x01,
        ]))?;
        io.feature(&cerberus_report(&[
            0x07,
            0x0B,
            PROFILE,
            CERBERUS_CUSTOM,
            0,
            0,
            0,
        ]))?;
        io.feature(&cerberus_report(&[
            0x07,
            0x0C,
            PROFILE,
            CERBERUS_BRIGHTNESS,
        ]))
    }
    /// SetPerLEDColor per key, then SendPerLEDColorEnd.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (key, c) in CERBERUS_KEYS.iter().zip(colors) {
            io.feature(&cerberus_report(&[
                0x07, 0x0D, PROFILE, *key, c.0, c.1, c.2,
            ]))?;
        }
        io.feature(&cerberus_report(&[0x07, 0x0D, PROFILE, 0xFF, 0, 0, 0]))
    }
}

/// The Sagaris has effects only; Static at colour slot 0 carries the push colour.
struct Sagaris;

impl Proto for Sagaris {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let c = colors[0];
        io.write(&report(&[
            0x06,
            0x0B,
            0x04,
            0x00,
            0x00,
            c.0 / 16,
            c.1 / 16,
            c.2 / 16,
        ]))?;
        flush(io)?;
        io.write(&report(&[
            0x06,
            0x0B,
            0x01,
            SAGARIS_STATIC,
            SAGARIS_BRIGHTNESS,
            0x00,
            0x00,
        ]))?;
        let mut r = [0u8; 65];
        io.read(&mut r, 20)?;
        Ok(())
    }
}

/// The Strix Claw only switches its scroll wheel and logo on or off.
struct StrixClaw;

impl Proto for StrixClaw {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let on = |c: Color| c != Color::BLACK;
        io.feature(&[0x07, 0x07, 0x01, on(colors[0]) as u8, 0, 0, 0, 0, 0])?;
        let logo = if on(colors[1]) { 255 } else { 0 };
        io.feature(&[0x07, 0x0A, 0x01, 0x00, logo, 0, 0, 0, 0])
    }
}

/// SetProfile(1), from the RGBController constructor.
pub fn set_profile(io: &mut dyn Io) -> Result<(), String> {
    io.feature(&cerberus_report(&[0x07, 0x03, PROFILE]))
}

fn cerberus(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    set_profile(io.as_mut())?;
    let dev = mapped(
        info,
        FAMILY,
        d,
        DeviceKind::Keyboard,
        &CERBERUS_MAP,
        CERBERUS_KEYS.len(),
    );
    Ok(Box::new(Driver::new(io, dev, FAMILY, Cerberus)))
}

fn sagaris(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Keyboard, 1);
    boxed(api, info, FAMILY, dev, Sagaris)
}

fn strix_claw(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 2);
    boxed(api, info, FAMILY, dev, StrixClaw)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "ASUS Cerberus Mech",
        LEGACY_VID,
        0x2047,
        1,
        0xFF01,
        1,
        cerberus,
    ),
    det(
        "ASUS Sagaris GK1100",
        ASUS_VID,
        0x1835,
        1,
        0xFF02,
        2,
        sagaris,
    ),
    det(
        "ASUS ROG Strix Claw",
        LEGACY_VID,
        0x1016,
        0,
        0xFF01,
        1,
        strix_claw,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AsusCerberusKeyboard: DeviceUpdateMode(Custom), then DeviceUpdateLEDs
    #[test]
    fn cerberus_start_and_frame() {
        let mut rec = Rec::default();
        set_profile(&mut rec).unwrap();
        assert_eq!(rec.take(), vec![Op::F(vec![7, 3, 1, 0, 0, 0, 0, 0])]);
        let mut a = vec![Color::BLACK; 105];
        a[0] = Color(1, 2, 3);
        let [first, _, next] = pushes(Cerberus, 105, vec![], &a, &[]);
        assert_eq!(first.len(), 4 + 3 + 105 + 1);
        assert_eq!(first[0], Op::F(vec![7, 0x0E, 1, 0, 0, 0, 0, 0]));
        assert_eq!(first[3], Op::F(vec![7, 0x0E, 1, 3, 0, 0, 0, 0]));
        assert_eq!(first[4], Op::F(vec![7, 0x0A, 1, 7, 1, 0, 0, 0]));
        assert_eq!(first[5], Op::F(vec![7, 0x0B, 1, 7, 0, 0, 0, 0]));
        assert_eq!(first[6], Op::F(vec![7, 0x0C, 1, 4, 0, 0, 0, 0]));
        assert_eq!(first[7], Op::F(vec![7, 0x0D, 1, 0x0B, 1, 2, 3, 0]));
        assert_eq!(first[111], Op::F(vec![7, 0x0D, 1, 0x72, 0, 0, 0, 0]));
        assert_eq!(first[112], Op::F(vec![7, 0x0D, 1, 0xFF, 0, 0, 0, 0]));
        assert_eq!(next.len(), 106);
    }

    // AsusSagarisKeyboardController::SetColor and SetMode for Static
    #[test]
    fn sagaris_static() {
        let [first, _, _] = pushes(Sagaris, 1, vec![], &[Color(0xFF, 0x80, 0x0F)], &[]);
        assert_eq!(
            first,
            vec![
                w(pad(&[6, 0x0B, 4, 0, 0, 0x0F, 0x08, 0x00], 65)),
                Op::Read,
                w(pad(&[6, 0x0B, 1, 1, 3, 0, 0], 65)),
                Op::Read,
            ]
        );
    }

    // StrixClawController::SetScrollWheelLED and SetLogoLED
    #[test]
    fn strix_claw_on_off() {
        let [first, _, _] = pushes(StrixClaw, 2, vec![], &[Color(1, 0, 0), Color::BLACK], &[]);
        assert_eq!(
            first,
            vec![
                Op::F(vec![7, 7, 1, 1, 0, 0, 0, 0, 0]),
                Op::F(vec![7, 0x0A, 1, 0, 0, 0, 0, 0, 0]),
            ]
        );
    }
}
