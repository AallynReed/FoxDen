//! Patriot Viper V380 headset (protocol learned from SignalRGB's V380_Headset.js).

use super::{HEADSET, boxed, det, row};
use crate::hid::{Io, Proto};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const LEN: usize = 16;
/// Static mode; written before every colour.
const STATIC: [u8; 7] = [0xFF, 0x03, 0x01, 0x01, 0x00, 0x0A, 0x00];

pub static DETECTORS: &[Detector] = &[det(
    "Patriot Viper V380",
    0x260D,
    0x2002,
    Some(3),
    None,
    Some(0x01),
    open,
)];

/// `sendColors`: the colour as G, B, R.
pub fn color(c: Color) -> [u8; 7] {
    [0xFF, 0x04, 0x00, 0x00, c.1, c.2, c.0]
}

struct V380;

impl Proto for V380 {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[0xFF, 0x02, 0x00, 0x00, 0x01, 0x00, 0x01], LEN)?;
        io.write(&STATIC, LEN)?;
        io.write(&[0xFF, 0x04, 0x00, 0x00, 0xFF, 0x00, 0xFF], LEN)?;
        io.write(&[0xFF, 0x01], LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        io.write(&STATIC, LEN)?;
        io.write(&color(c[0]), LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0)]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = row(info, "patriot-v380", d.name, DeviceKind::Other, HEADSET, 1);
    boxed(api, info, "patriot v380", dev, V380)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gbr_order() {
        assert_eq!(color(Color(1, 2, 3))[4..], [2, 3, 1]);
    }
}
