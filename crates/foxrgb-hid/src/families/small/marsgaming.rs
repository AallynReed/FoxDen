//! Mars Gaming MM4 mouse LED, set live on the active profile.

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "marsgaming";

/// Feature report 0x02: inverted colour, brightness 3, breathing off.
pub fn color(c: Color) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&[0x02, 0x04, !c.0, !c.1, !c.2, 0x03, 0x00, 0x01]);
    b
}

struct Mm4;

impl Proto for Mm4 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&color(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    if info.usage_page < 0xFF00 {
        return Err("not the vendor collection".into());
    }
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Mm4)
}

pub static DETECTORS: &[Detector] = &[det("Mars Gaming MM4", 0x04D9, 0xFA58, ANY, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn inverted_colour_report() {
        let [first, _, next] = pushes(Mm4, 1, vec![], &[Color(0, 0x10, 0xFF)], &[Color::BLACK]);
        assert_eq!(
            first,
            vec![Op::F(pad(
                &[0x02, 0x04, 0xFF, 0xEF, 0x00, 0x03, 0x00, 0x01],
                16
            ))]
        );
        assert_eq!(
            next,
            vec![Op::F(pad(
                &[0x02, 0x04, 0xFF, 0xFF, 0xFF, 0x03, 0x00, 0x01],
                16
            ))]
        );
    }
}
