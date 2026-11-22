// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, NA, at, backend, device, matrix, on_ipu, open_io};
use crate::registry::{Detector, Info};

const KB_LEDS: usize = 113;
const PER_PACKET: usize = 14;

#[rustfmt::skip]
const KEYVALUE: [u8; KB_LEDS] = [
      0,   9,  18,  27,  36,  45,  54,  63,  72,  81,
     90,  99, 108, 117, 126,  15, 135, 140, 141, 142,
      1,  10,  19,  28,  37,  46,  55,  64,  73,  82,
     91, 100, 109, 127, 128,  24,  33,  42,  51,  60,
     69,   2,  11,  20,  29,  38,  47,  56,  65,  74,
     83,  92, 101, 110, 119, 129,  78,  87,  96, 105,
     16,  25,   3,  12,  21,  30,  39,  48,  57,  66,
     75,  84,  93, 102, 120,  34,  43,  52,   4,  22,
     31,  40,  49,  58,  67,  76,  85,  94, 103, 121,
    130,  70,  79,  88, 106,   5,  14,  23,  50,  86,
     95, 104, 113, 122, 131, 132, 133,  97, 147, 148,
    149, 150, 151,
];

#[rustfmt::skip]
const MAP: [u32; 6 * 23] = [
     NA,   0,  NA,   1,   2,   3,   4,   5,   6,   7,   8,  NA,   9,  10,  11,  12,  13,  14,  15,  16,  17,  18,  19,
    108,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,  33,  NA,  34,  35,  36,  37,  38,  39,  40,
    109,  41,  NA,  42,  43,  44,  45,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,  56,  57,  58,  59,  60,  61,
    110,  62,  NA,  63,  64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  NA,  74,  NA,  NA,  NA,  75,  76,  77,  NA,
    111,  78,  NA,  79,  80,  81,  82,  83,  84,  85,  86,  87,  88,  NA,  89,  NA,  NA,  90,  NA,  91,  92,  93,  94,
    112,  95,  96,  97,  NA,  NA,  NA,  98,  NA,  NA,  NA,  99, 100,  NA, 101, 102, 103, 104, 105, 106,  NA, 107,  NA,
];

/// `CougarKeyboardController::SetLedsDirect`: 14 keys per report, flagged 1 first, 2 after and
/// 3 on a short last one. The buffer is reused, so a short report keeps earlier keys' bytes.
fn leds_direct(colors: &[Color]) -> Vec<Vec<u8>> {
    let total = colors.len();
    let mut max = PER_PACKET;
    let mut buf = vec![0u8; 65];
    buf[..9].copy_from_slice(&[0x00, 0x14, 0x2C, 0x0B, 0x00, 0xFF, 0x64, 0x00, 0x01]);
    let mut out = Vec::new();
    let mut sent = 0;
    while sent < total {
        if sent + max > total {
            max = total - sent;
            buf[8] = 3;
        }
        for i in 0..max {
            let o = 9 + i * 4;
            let c = colors[sent + i];
            buf[o..o + 4].copy_from_slice(&[KEYVALUE[sent + i], c.0, c.1, c.2]);
        }
        out.push(buf.clone());
        buf[8] = 2;
        sent += PER_PACKET;
    }
    out
}

struct Keyboard;

impl Driver for Keyboard {
    // RGBController_CougarKeyboard::DeviceUpdateLEDs; Direct needs no mode packet.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let colors: Vec<Color> = (0..KB_LEDS).map(|i| at(colors, i)).collect();
        for r in leds_direct(&colors) {
            io.write(&r)?;
            io.sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}

/// Revenger ST zone addresses for Direct: mode, colour (R, G, B follow) and brightness.
const MODE_ADDR: [u8; 3] = [0x77, 0x55, 0xE6];
const MODE_BYTE: [u8; 3] = [0x01, 0x00, 0x00];
const RGB_ADDR: [u8; 3] = [0x79, 0x57, 0xE8];
const BRIGHT_ADDR: [u8; 3] = [0x78, 0x56, 0xE7];
const DIRECT: u8 = 0x01;
const MAX_BRIGHTNESS: u8 = 0xFF;

fn st_report(addr: u8, byte5: u8, value: u8) -> Vec<u8> {
    vec![0x00, 0xC4, 0x0F, 0x00, addr, byte5, value, 0x00, 0x00]
}

struct RevengerSt;

impl Driver for RevengerSt {
    // CougarRevengerSTController constructor: ActivateMode(zone, DIRECT) for the three zones.
    fn setup(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for z in 0..3 {
            io.feature(&st_report(MODE_ADDR[z], MODE_BYTE[z], DIRECT))?;
        }
        Ok(())
    }

    // RGBController_CougarRevengerST::DeviceUpdateLEDs: SetDirect per zone at full brightness.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for z in 0..3 {
            let c = at(colors, z);
            for (k, v) in [c.0, c.1, c.2].into_iter().enumerate() {
                io.feature(&st_report(RGB_ADDR[z] + k as u8, MODE_BYTE[z], v))?;
            }
            io.feature(&st_report(BRIGHT_ADDR[z], MODE_BYTE[z], MAX_BRIGHTNESS))?;
        }
        Ok(())
    }
}

fn open_700k(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = open_io(api, info)?;
    let leds = matrix(&MAP, 23, KB_LEDS, |_| [0.5, 0.5]);
    let dev = device(
        info.device_id("cougar"),
        d.name,
        DeviceKind::Keyboard,
        [23, 6],
        leds,
    );
    Ok(backend(io, dev, Keyboard, "cougar keyboard"))
}

fn open_revenger(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = open_io(api, info)?;
    // Bottom, mouse wheel, logo.
    let dev = device(
        info.device_id("cougar"),
        d.name,
        DeviceKind::Mouse,
        [1, 3],
        vec![[0.5, 0.95], [0.5, 0.15], [0.5, 0.7]],
    );
    Ok(backend(io, dev, RevengerSt, "cougar revenger st"))
}

pub(super) static DETECTORS: [Detector; 2] = [
    on_ipu(
        "Cougar 700K EVO Gaming Keyboard",
        0x060B,
        0x7010,
        3,
        0xFF00,
        1,
        open_700k,
    ),
    on_ipu(
        "Cougar Revenger ST",
        0x12CF,
        0x0412,
        0,
        0x0001,
        2,
        open_revenger,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op, ms};
    use super::*;

    // CougarKeyboardController::SetLedsDirect for all 113 keys
    #[test]
    fn keyboard_frame() {
        let colors: Vec<Color> = (0..KB_LEDS as u8).map(|i| Color(i, i, 1)).collect();
        let mut fake = Fake::default();
        Keyboard.frame(&mut fake, &colors).unwrap();
        let ops = fake.ops();
        assert_eq!(ops.len(), 9 * 2);
        assert!(ops.iter().skip(1).step_by(2).all(|o| *o == ms(1)));
        let reports: Vec<&Vec<u8>> = ops
            .iter()
            .filter_map(|o| match o {
                Op::Write(b) => Some(b),
                _ => None,
            })
            .collect();
        let flags: Vec<u8> = reports.iter().map(|r| r[8]).collect();
        assert_eq!(flags, [1, 2, 2, 2, 2, 2, 2, 2, 3]);
        assert_eq!(
            reports[0][..13],
            [0, 0x14, 0x2C, 0x0B, 0, 0xFF, 0x64, 0, 1, 0, 0, 0, 1]
        );
        assert_eq!(reports[1][9..13], [126, 14, 14, 1]);
        // The last report holds key 112 (G5), then key 99..111's bytes left from the one before.
        let mut last = reports[7].clone();
        last[8] = 3;
        last[9..13].copy_from_slice(&[151, 112, 112, 1]);
        assert_eq!(*reports[8], last);
    }

    // CougarRevengerSTController::ActivateMode(zone, DIRECT_MODE_VALUE) x3
    #[test]
    fn revenger_setup() {
        let mut fake = Fake::default();
        RevengerSt.setup(&mut fake).unwrap();
        assert_eq!(
            fake.ops(),
            vec![
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0x77, 0x01, 0x01, 0, 0]),
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0x55, 0x00, 0x01, 0, 0]),
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0xE6, 0x00, 0x01, 0, 0]),
            ]
        );
    }

    // CougarRevengerSTController::SetDirect(zone, color, 0xFF) for each zone
    #[test]
    fn revenger_frame() {
        let mut fake = Fake::default();
        RevengerSt
            .frame(&mut fake, &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)])
            .unwrap();
        let ops = fake.ops();
        assert_eq!(ops.len(), 12);
        assert_eq!(
            ops[..4],
            [
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0x79, 0x01, 1, 0, 0]),
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0x7A, 0x01, 2, 0, 0]),
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0x7B, 0x01, 3, 0, 0]),
                Op::Feature(vec![0, 0xC4, 0x0F, 0, 0x78, 0x01, 0xFF, 0, 0]),
            ]
        );
        assert_eq!(
            ops[8],
            Op::Feature(vec![0, 0xC4, 0x0F, 0, 0xE8, 0x00, 7, 0, 0])
        );
        assert_eq!(
            ops[11],
            Op::Feature(vec![0, 0xC4, 0x0F, 0, 0xE7, 0x00, 0xFF, 0, 0])
        );
    }
}
