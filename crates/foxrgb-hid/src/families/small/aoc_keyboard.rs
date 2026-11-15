//! Ported from OpenRGB Controllers/AOCKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::klm::{FULL, Klm, Layout};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "aoc-keyboard";
const CUSTOM: u8 = 0x09;
const BRIGHTNESS_HIGH: u8 = 0x03;
/// Direct's direction is 0, MODE_DIRECTION_LEFT, which DeviceUpdateMode turns into counter-clockwise.
const COUNTERCLOCKWISE: u8 = 0x01;

const OFFSETS: [u32; 106] = [
    90, 92, 77, 63, 79, 94, 81, 96, 82, 83, 98, 40, 55, 85, 100, 104, 75, 76, 91, 62, 48, 64, 50,
    65, 66, 67, 97, 68, 84, 70, 59, 74, 89, 58, 73, 88, 103, 60, 61, 47, 78, 33, 49, 35, 80, 51,
    52, 53, 69, 99, 25, 44, 29, 14, 43, 28, 13, 102, 45, 46, 32, 93, 18, 34, 20, 95, 36, 37, 38,
    54, 0, 10, 57, 72, 87, 30, 0, 31, 17, 2, 3, 19, 5, 6, 21, 22, 23, 39, 11, 42, 27, 12, 101, 15,
    0, 1, 4, 7, 8, 24, 9, 26, 41, 56, 71, 86,
];

fn keyboard() -> Klm {
    Klm::new(Layout::AnsiQwerty, FULL, &OFFSETS, &[])
}

fn marker(cmd: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x09;
    b[1] = cmd;
    b
}

pub fn lighting_config(
    mode: u8,
    random: u8,
    brightness: u8,
    speed: u8,
    direction: u8,
    c: Color,
) -> [u8; 117] {
    let mut b = [0u8; 117];
    b[0] = 0x14;
    b[1] = 0x01;
    b[6] = mode;
    let at = 7 + 9 * mode as usize;
    b[at..at + 7].copy_from_slice(&[c.0, c.1, c.2, random, direction, speed, brightness]);
    let sum = b[..115]
        .iter()
        .fold(0x4A9Eu16, |s, &v| s.wrapping_add(v as u16));
    b[115] = sum as u8;
    b[116] = (sum >> 8) as u8;
    b
}

/// SendCustomPacket: 120 slots, all reds, then greens, then blues.
pub fn custom(slots: &[Color; 120]) -> [u8; 361] {
    let mut b = [0u8; 361];
    b[0] = 0x20;
    for (i, c) in slots.iter().enumerate() {
        b[1 + i] = c.0;
        b[121 + i] = c.1;
        b[241 + i] = c.2;
    }
    b
}

struct AocKeyboard {
    values: Vec<u32>,
    moded: bool,
}

impl Proto for AocKeyboard {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if !self.moded {
            io.write(&marker(0x21))?;
            io.sleep(5);
            io.feature(&lighting_config(
                CUSTOM,
                0,
                BRIGHTNESS_HIGH,
                0,
                COUNTERCLOCKWISE,
                colors[0],
            ))?;
            io.sleep(5);
            io.write(&marker(0x22))?;
            io.sleep(10);
            self.moded = true;
        }
        let mut slots = [Color::BLACK; 120];
        for (&v, &c) in self.values.iter().zip(colors) {
            slots[v as usize] = c;
        }
        io.write(&marker(0x21))?;
        io.sleep(5);
        io.feature(&custom(&slots))?;
        io.sleep(5);
        io.write(&marker(0x22))?;
        io.sleep(5);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let kb = keyboard();
    let proto = AocKeyboard {
        values: kb.values(),
        moded: false,
    };
    boxed(api, info, FAMILY, kb.device(info, FAMILY, d), proto)
}

pub static DETECTORS: &[Detector] = &[
    det("AOC GK500", 0x3938, 0x1178, ANY, 0xFF19, 0xFF19, open),
    det("AOC GK500", 0x3938, 0x1229, ANY, 0xFF19, 0xFF19, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DeviceUpdateMode(Direct) -> SetLightingConfig, then DeviceUpdateLEDs -> SetCustom
    #[test]
    fn config_then_custom() {
        let kb = keyboard();
        let n = kb.keys.len();
        assert_eq!(n, 104);
        let mut a = vec![Color::BLACK; n];
        a[0] = Color(1, 2, 3); // Escape, slot 90
        let mut b = a.clone();
        b[0] = Color(4, 5, 6);
        let proto = AocKeyboard {
            values: kb.values(),
            moded: false,
        };
        let [first, _, next] = pushes(proto, n, vec![], &a, &b);
        let start = w(pad(&[0x09, 0x21], 64));
        let end = w(pad(&[0x09, 0x22], 64));
        let mut cfg = pad(&[0x14, 0x01, 0, 0, 0, 0, 0x09], 117);
        cfg[88..95].copy_from_slice(&[1, 2, 3, 0, 1, 0, 3]);
        let sum = 0x4A9E + 0x14 + 1 + 9 + 1 + 2 + 3 + 1 + 3;
        cfg[115] = sum as u8;
        cfg[116] = (sum >> 8) as u8;
        let mut cus = pad(&[0x20], 361);
        cus[1 + 90] = 1;
        cus[121 + 90] = 2;
        cus[241 + 90] = 3;
        assert_eq!(
            first,
            vec![
                start.clone(),
                Op::Sleep(5),
                Op::F(cfg),
                Op::Sleep(5),
                end.clone(),
                Op::Sleep(10),
                start.clone(),
                Op::Sleep(5),
                Op::F(cus.clone()),
                Op::Sleep(5),
                end.clone(),
                Op::Sleep(5)
            ]
        );
        cus[1 + 90] = 4;
        cus[121 + 90] = 5;
        cus[241 + 90] = 6;
        assert_eq!(next[2], Op::F(cus));
        assert_eq!(next.len(), 6);
    }
}
