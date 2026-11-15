//! Ported from OpenRGB Controllers/LegoDimensionsToypadBaseController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "lego-toypad";
const REPORT: u8 = 0x55;

pub fn activate() -> [u8; 32] {
    let mut b = [0u8; 32];
    b[1..19].copy_from_slice(&[
        REPORT, 0x0F, 0xB0, 0x01, 0x28, 0x63, 0x29, 0x20, 0x4C, 0x45, 0x47, 0x4F, 0x20, 0x32, 0x30,
        0x31, 0x34, 0xF7,
    ]);
    b
}

pub fn set_direct(zone: u8, c: Color) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[1..9].copy_from_slice(&[REPORT, 0x06, 0xC0, 0x02, zone, c.0, c.1, c.2]);
    b[9] = b[1..9].iter().fold(0u8, |a, &v| a.wrapping_add(v));
    b
}

struct Toypad;

impl Proto for Toypad {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&activate())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        (0..3).try_for_each(|z| io.write(&set_direct(z as u8 + 1, colors[z])))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Other,
        3,
        1,
        &[(1, 0), (0, 0), (2, 0)],
    );
    boxed(api, info, FAMILY, dev, Toypad)
}

pub static DETECTORS: &[Detector] = &[det(
    "Lego Dimensions Toypad Base",
    0x0E6F,
    0x0241,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // Activate from the constructor, then SetDirect for zones 1..3
    #[test]
    fn activate_then_zones() {
        let a = [
            Color(0x10, 0x20, 0x30),
            Color(0, 0, 0),
            Color(255, 255, 255),
        ];
        let [first, _, _] = pushes(Toypad, 3, vec![], &a, &[]);
        assert_eq!(first.len(), 4);
        assert_eq!(first[0], w(activate().to_vec()));
        assert_eq!(
            first[0],
            w(pad(
                &[
                    0, 0x55, 0x0F, 0xB0, 1, 0x28, 0x63, 0x29, 0x20, 0x4C, 0x45, 0x47, 0x4F, 0x20,
                    0x32, 0x30, 0x31, 0x34, 0xF7
                ],
                32
            ))
        );
        // 0x55 + 6 + 0xC0 + 2 + 1 + 0x60 = 0x17E
        assert_eq!(
            first[1],
            w(pad(&[0, 0x55, 6, 0xC0, 2, 1, 0x10, 0x20, 0x30, 0x7E], 32))
        );
        assert_eq!(
            first[2],
            w(pad(&[0, 0x55, 6, 0xC0, 2, 2, 0, 0, 0, 0x1F], 32))
        );
        // 0x120 + 3 * 255 = 0x41D
        assert_eq!(
            first[3],
            w(pad(&[0, 0x55, 6, 0xC0, 2, 3, 255, 255, 255, 0x1D], 32))
        );
    }
}
