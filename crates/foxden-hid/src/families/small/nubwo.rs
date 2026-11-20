//! Nubwo X7 Spectrum mouse LED.

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "nubwo";

/// Feature report 0x03: colour, static on (off for black), brightness 3, slow tempo.
pub fn color(c: Color) -> [u8; 64] {
    let on = u8::from(c != Color::BLACK);
    let mut b = [0u8; 64];
    b[..16].copy_from_slice(&[
        0x03, 0x06, 0xBB, 0xAA, 0x2A, 0x00, 0x0A, 0x00, c.0, c.1, c.2, on, 0x00, 0x03, 0x01, 0x00,
    ]);
    b
}

struct X7;

impl Proto for X7 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&color(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    if info.usage_page < 0xFF00 {
        return Err("not the vendor collection".into());
    }
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, X7)
}

pub static DETECTORS: &[Detector] = &[det(
    "Nubwo X7 Spectrum",
    0x258A,
    0x0012,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn colour_report() {
        let [first, _, next] = pushes(X7, 1, vec![], &[Color(1, 2, 3)], &[Color::BLACK]);
        let head = [0x03, 0x06, 0xBB, 0xAA, 0x2A, 0x00, 0x0A, 0x00];
        let mut want = head.to_vec();
        want.extend([1, 2, 3, 0x01, 0x00, 0x03, 0x01]);
        assert_eq!(first, vec![Op::F(pad(&want, 64))]);
        let mut off = head.to_vec();
        off.extend([0, 0, 0, 0x00, 0x00, 0x03, 0x01]);
        assert_eq!(next, vec![Op::F(pad(&off, 64))]);
    }
}
