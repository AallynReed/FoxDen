//! Ported from OpenRGB Controllers/CreativeController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "creative-xg6";

fn packet(head: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..head.len()].copy_from_slice(head);
    b
}

/// SetLedColor at full brightness; the colour goes out as BGR.
pub fn colour_packets(c: Color) -> [[u8; 64]; 3] {
    let [r, g, b] = rgb(c);
    [
        packet(&[0, 0x5A, 0x3A, 0x02, 0x06, 0x01]),
        packet(&[0, 0x5A, 0x3A, 0x06, 0x04, 0, 0x03, 0x01, 0, 0x01]),
        packet(&[
            0, 0x5A, 0x3A, 0x09, 0x0A, 0, 0x03, 0x01, 0x01, 0xFF, b, g, r,
        ]),
    ]
}

struct Xg6;

impl Proto for Xg6 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in colour_packets(colors[0]) {
            io.write(&p)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, Xg6)
}

pub static DETECTORS: &[Detector] = &[det(
    "Creative SoundBlasterX G6",
    0x041E,
    0x3256,
    4,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn colour_sequence() {
        let [first, _, _] = pushes(Xg6, 1, vec![], &[Color(1, 2, 3)], &[]);
        assert_eq!(
            first,
            vec![
                w(pad(&[0, 0x5A, 0x3A, 2, 6, 1], 64)),
                w(pad(&[0, 0x5A, 0x3A, 6, 4, 0, 3, 1, 0, 1], 64)),
                w(pad(
                    &[0, 0x5A, 0x3A, 9, 0x0A, 0, 3, 1, 1, 0xFF, 3, 2, 1],
                    64
                )),
            ]
        );
    }
}
