//! OEM wireless keyboard 3554:FA09 (protocol learned from SignalRGB's
//! OEM_Keyboard_Wireless_Controller.js).

use super::{KEYBOARD, boxed, det, row};
use crate::hid::{Io, Proto};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const LEN: usize = 20;

pub static DETECTORS: &[Detector] = &[det(
    "OEM Wireless Keyboard",
    0x3554,
    0xFA09,
    Some(1),
    Some(0xFF02),
    Some(0x02),
    open,
)];

/// `writeRGBPackage`: report 0x13, one colour, byte 19 the low byte of the sum of bytes 0..19.
pub fn color(c: Color) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[..8].copy_from_slice(&[0x13, 0x88, 0x01, 0x00, 0x23, c.0, c.1, c.2]);
    b[19] = b[..19].iter().fold(0u8, |a, &v| a.wrapping_add(v));
    b
}

struct Oem;

impl Proto for Oem {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        io.write(&color(c[0]), LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&color(Color(0, 0, 0)), LEN);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = row(
        info,
        "oem-wireless-kb",
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        1,
    );
    boxed(api, info, "oem keyboard", dev, Oem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum() {
        let b = color(Color(0xFF, 0x10, 0x01));
        assert_eq!(
            b[19],
            ((0x13 + 0x88 + 0x01 + 0x23 + 0xFF + 0x10 + 0x01) & 0xFF) as u8
        );
    }
}
