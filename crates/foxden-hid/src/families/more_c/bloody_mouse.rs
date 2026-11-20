//! Ported from OpenRGB Controllers/A4TechController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "bloody-mouse";
const W60_PRO_PID: u16 = 0x37EA;
const MP_50RS_PID: u16 = 0xFA60;

/// LED values in zone order: scroll wheel, mid line, logo, rear.
const W60_PRO: [u8; 15] = [14, 0, 1, 2, 9, 3, 4, 5, 6, 10, 8, 7, 13, 12, 11];
/// Scroll wheel, logo, rear.
const W90_MAX: [u8; 15] = [14, 7, 13, 12, 11, 10, 9, 8, 6, 5, 4, 3, 2, 1, 0];
const MP_50RS: [u8; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

fn layout(pid: u16) -> &'static [u8] {
    match pid {
        W60_PRO_PID => &W60_PRO,
        MP_50RS_PID => &MP_50RS,
        _ => &W90_MAX,
    }
}

/// SetLedsDirect: each LED at byte 8 + 3 * its value.
pub fn direct(values: &[u8], colors: &[Color]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..4].copy_from_slice(&[0x07, 0x03, 0x06, 0x02]);
    for (&v, c) in values.iter().zip(colors) {
        let at = 8 + 3 * v as usize;
        b[at..at + 3].copy_from_slice(&rgb(*c));
    }
    b
}

struct Bloody(&'static [u8]);

impl Proto for Bloody {
    // InitDevice from the constructor
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        let mut b = [0u8; 64];
        b[..4].copy_from_slice(&[0x07, 0x03, 0x06, 0x01]);
        io.feature(&b)?;
        b[3] = 0;
        b[8] = 1;
        io.feature(&b)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&direct(self.0, colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let values = layout(info.pid);
    let kind = if info.pid == MP_50RS_PID {
        DeviceKind::Other
    } else {
        DeviceKind::Mouse
    };
    let dev = line(info, FAMILY, d, kind, values.len());
    boxed(api, info, FAMILY, dev, Bloody(values))
}

pub static DETECTORS: &[Detector] = &[
    det(
        "Bloody W60 Pro",
        0x09DA,
        W60_PRO_PID,
        2,
        0xFF33,
        0x0529,
        open,
    ),
    det("Bloody W70 Max", 0x09DA, 0x79EF, 2, 0xFF33, 0x0518, open),
    det("Bloody W90 Max", 0x09DA, 0x3666, 2, 0xFF33, 0x053D, open),
    det("Bloody W90 Pro", 0x09DA, 0x39B6, 2, 0xFF33, 0x054D, open),
    det(
        "Bloody MP 50RS",
        0x09DA,
        MP_50RS_PID,
        2,
        0xFFF2,
        0x6009,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layouts() {
        assert_eq!(layout(W60_PRO_PID).len(), 15);
        assert_eq!(layout(0x79EF).len(), 15);
        assert_eq!(layout(0x3666).len(), 15);
        assert_eq!(layout(0x39B6).len(), 15);
        assert_eq!(layout(MP_50RS_PID).len(), 10);
    }

    #[test]
    fn init_then_direct() {
        let mut a = vec![Color::BLACK; 15];
        a[0] = Color(1, 2, 3); // value 14: bytes 50..53
        a[1] = Color(4, 5, 6); // value 0: bytes 8..11
        let [first, _, _] = pushes(Bloody(&W60_PRO), 15, vec![], &a, &[]);
        assert_eq!(first[0], Op::F(pad(&[7, 3, 6, 1], 64)));
        assert_eq!(first[1], Op::F(pad(&[7, 3, 6, 0, 0, 0, 0, 0, 1], 64)));
        let mut p = pad(&[7, 3, 6, 2, 0, 0, 0, 0, 4, 5, 6], 64);
        p[50..53].copy_from_slice(&[1, 2, 3]);
        assert_eq!(first[2], Op::F(p));
        assert_eq!(first.len(), 3);
    }
}
