//! Mountain Macropad and Makalu 67 mouse.

use super::layouts::{MACROPAD_POS, MAKALU_POS};
use super::{KEYBOARD, MOUSE, boxed, det, placed};
use crate::hid::{Io, Proto, drain, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x3282;

pub static DETECTORS: &[Detector] = &[
    det(
        "Mountain Macropad",
        VID,
        0x0008,
        Some(2),
        None,
        None,
        open_macropad,
    ),
    det(
        "Mountain Makalu 67",
        VID,
        0x0003,
        Some(1),
        Some(0xFF01),
        Some(0x02),
        open_makalu,
    ),
];

/// `StreamLightingData`: twelve keys, RGB each.
pub fn macropad(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0x00, 0x14, 0x2C, 0x00, 0x01, 0x00, 0x4B, 0x00];
    b.extend(colors.iter().take(MACROPAD_POS.len()).flat_map(|&c| rgb(c)));
    b
}

/// The third start report, sent on report id 0x14.
fn macropad_unlock() -> Vec<u8> {
    let mut b = vec![0x14, 0x2C, 0x0A, 0x00, 0xFF, 0x64];
    b.resize(64, 0xFF);
    b
}

struct Macropad;

impl Proto for Macropad {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[0x00, 0x11], 65)?;
        io.write(&[0x00, 0x14, 0x00, 0x00, 0x00, 0x01, 0x06], 65)?;
        let _ = io.write(&macropad_unlock(), 64);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        drain(io, 50);
        io.write(&macropad(c), 65)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&macropad(&[Color(0, 0, 0); 12]), 65);
    }
}

fn open_macropad(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        "mountain-macropad",
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        [6, 2],
        MACROPAD_POS,
    );
    boxed(api, info, "mountain macropad", dev, Macropad)
}

/// Makalu feature report A1 3C: `mode` at 16, colours from 17, 0x64 at 41.
pub fn makalu(mode: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0xA1;
    b[1] = 0x3C;
    b[16] = mode;
    b[41] = 0x64;
    for (i, &c) in colors.iter().take(MAKALU_POS.len()).enumerate() {
        b[17 + i * 3..20 + i * 3].copy_from_slice(&rgb(c));
    }
    b
}

struct Makalu;

impl Proto for Makalu {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&makalu(0x01, &[]), 64)
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        io.feature(&makalu(0x0F, c), 64)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(&makalu(0x0F, &[]), 64);
    }
}

fn open_makalu(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        "mountain-makalu",
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        [3, 5],
        MAKALU_POS,
    );
    boxed(api, info, "mountain makalu", dev, Makalu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports() {
        let m = macropad(&[Color(1, 2, 3); 12]);
        assert_eq!(m.len(), 8 + 36);
        assert_eq!(m[8..11], [1, 2, 3]);
        let k = makalu(0x0F, &[Color(4, 5, 6); 8]);
        assert_eq!(k[16..20], [0x0F, 4, 5, 6]);
        assert_eq!(k[38..42], [4, 5, 6, 0x64]);
        assert_eq!(macropad_unlock().len(), 64);
    }
}
