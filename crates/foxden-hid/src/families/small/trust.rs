//! Ported from OpenRGB Controllers/TrustController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "trust";

/// TrustGXT114Controller::SetMode in Static, which has no per-LED mode; the push colour drives it.
pub fn gxt114_static(c: Color) -> [u8; 154] {
    let mut b = [0u8; 154];
    b[0] = 0x04;
    b[93] = 0x28;
    b[96] = 0xA2;
    b[103..106].copy_from_slice(&rgb(c));
    b[79..=89].fill(0x80);
    for at in [
        101, 108, 109, 113, 114, 115, 117, 118, 119, 124, 127, 128, 131, 134, 135, 138, 139, 141,
        142, 143, 144,
    ] {
        b[at] = 0xFF;
    }
    for (at, v) in [
        (8, 0x4C),
        (71, 0x11),
        (72, 0xB0),
        (74, 0x89),
        (75, 0x0E),
        (76, 0x9D),
        (77, 0xA7),
        (78, 0xB7),
        (148, 0x58),
        (149, 0x30),
        (150, 0x31),
        (151, 0x30),
        (152, 0x31),
        (153, 0x30),
    ] {
        b[at] = v;
    }
    b
}

/// TrustGXT180Controller::SetMode in Static (per-LED), full brightness, speed 0.
pub fn gxt180_static(c: Color) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..15].copy_from_slice(&[
        0x03, 0x06, 0xBB, 0xAA, 0x2A, 0, 0x0A, 0, c.0, c.1, c.2, 0x01, 0, 0x05, 0,
    ]);
    b
}

struct Trust {
    gxt114: bool,
}

impl Proto for Trust {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if self.gxt114 {
            io.feature(&gxt114_static(colors[0]))
        } else {
            io.feature(&gxt180_static(colors[0]))
        }
    }
}

fn open114(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    // DetectTrustGXT114Controllers keeps the device only if its feature report answers
    let mut probe = [0u8; 154];
    probe[0] = 0x04;
    if io.get_feature(&mut probe)? == 0 {
        return Err("no reply to the GXT 114 feature report".into());
    }
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    Ok(Box::new(Driver::new(
        io,
        dev,
        FAMILY,
        Trust { gxt114: true },
    )))
}

fn open180(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Trust { gxt114: false })
}

pub static DETECTORS: &[Detector] = &[
    det("Trust GXT 114", 0x145F, 0x026D, 1, 0xFF00, 1, open114),
    det("Trust GXT 180", 0x145F, 0x0248, 1, 0xFFA0, 1, open180),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // TrustGXT114Controller::SetMode(Static)
    #[test]
    fn gxt114() {
        let [first, _, _] = pushes(Trust { gxt114: true }, 1, vec![], &[Color(1, 2, 3)], &[]);
        let Op::F(b) = &first[0] else { panic!() };
        assert_eq!(b.len(), 154);
        assert_eq!(b[..9], [4, 0, 0, 0, 0, 0, 0, 0, 0x4C]);
        assert_eq!(
            b[71..80],
            [0x11, 0xB0, 0, 0x89, 0x0E, 0x9D, 0xA7, 0xB7, 0x80]
        );
        assert_eq!(b[89..97], [0x80, 0, 0, 0, 0x28, 0, 0, 0xA2]);
        assert_eq!(b[101..106], [0xFF, 0, 1, 2, 3]);
        assert_eq!(
            b[144..],
            [0xFF, 0, 0, 0, 0x58, 0x30, 0x31, 0x30, 0x31, 0x30]
        );
        assert_eq!(b.iter().filter(|&&v| v == 0xFF).count(), 21);
    }

    // TrustGXT180Controller::SetMode(Static)
    #[test]
    fn gxt180() {
        let [first, _, _] = pushes(Trust { gxt114: false }, 1, vec![], &[Color(1, 2, 3)], &[]);
        assert_eq!(
            first,
            vec![Op::F(pad(
                &[3, 6, 0xBB, 0xAA, 0x2A, 0, 0x0A, 0, 1, 2, 3, 1, 0, 5],
                64
            ))]
        );
    }
}
