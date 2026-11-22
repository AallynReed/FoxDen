//! MSI Vigor GK41/GK50/GK71 keyboards and the MSI MAG monitor light.

use super::layouts::MSI_MAG_POS;
use super::{KEYBOARD, MONITOR, boxed, det, placed, row};
use crate::hid::{Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const MSI: u16 = 0x0DB0;

const fn vigor(pid: u16, name: &'static str) -> Detector {
    det(
        name,
        MSI,
        pid,
        Some(1),
        Some(0xFF00),
        Some(0x01),
        open_vigor,
    )
}

pub static DETECTORS: &[Detector] = &[
    vigor(0x0B41, "MSI Vigor GK41 Dusk"),
    vigor(0x0B5B, "MSI Vigor GK50 Elite"),
    vigor(0x0B7A, "MSI Vigor GK71 Sonic"),
    vigor(0x1958, "MSI Vigor GK71 Sonic"),
    det(
        "MSI MAG Monitor",
        0x1038,
        0x1128,
        Some(0),
        Some(0xFFC0),
        Some(0x01),
        open_mag,
    ),
];

const VIGOR_LEN: usize = 65;
#[rustfmt::skip]
const VIGOR_HEAD: [u8; 49] = [
    0x00, 0x56, 0x21, 0x01, 0x00,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x0B, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x01, 0x00,
    0x00, 0x01, 0x00, 0xC1, 0x00, 0x00, 0x00, 0x00,
];
const VIGOR_TAIL: [u8; 13] = [
    0xFF, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x50, 0x30, 0x00, 0x00,
];
const VIGOR_APPLY: [u8; 6] = [0x00, 0x51, 0x28, 0x00, 0x00, 0x01];

/// `sendColors`: the static-colour profile with the one keyboard colour, then apply.
pub fn vigor_color(c: Color) -> Vec<u8> {
    let mut b = VIGOR_HEAD.to_vec();
    b.extend(rgb(c));
    b.extend(VIGOR_TAIL);
    b
}

struct Vigor;

impl Proto for Vigor {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[0x00, 0x41, 0x80], VIGOR_LEN)?;
        io.write(&[0x00, 0x56, 0x20, 0x01], VIGOR_LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        io.write(&vigor_color(c[0]), VIGOR_LEN)?;
        io.write(&VIGOR_APPLY, VIGOR_LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0)]);
    }
}

fn open_vigor(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = row(
        info,
        "msi-vigor-kb",
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        1,
    );
    boxed(api, info, "msi vigor", dev, Vigor)
}

const MAG_LEN: usize = 525;
const MAG_FIRST: usize = 40;

/// `sendColors`: (index, R, G, B) per LED, forty in the first feature report, the rest in the second.
pub fn mag_reports(colors: &[Color]) -> [Vec<u8>; 2] {
    let entry = |i: usize| {
        let c = colors.get(i).copied().unwrap_or(Color(0, 0, 0));
        [i as u8, c.0, c.1, c.2]
    };
    let report = |from: usize, to: usize| {
        let mut b = vec![0x00, 0x0C, 0x00, (to - from) as u8, 0x00];
        b.extend((from..to).flat_map(entry));
        b
    };
    [report(0, MAG_FIRST), report(MAG_FIRST, MSI_MAG_POS.len())]
}

struct Mag;

impl Proto for Mag {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        mag_reports(c)
            .iter()
            .try_for_each(|p| io.feature(p, MAG_LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[]);
    }
}

fn open_mag(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        "msi-mag-monitor",
        d.name,
        DeviceKind::Other,
        MONITOR,
        [40, 2],
        MSI_MAG_POS,
    );
    boxed(api, info, "msi mag monitor", dev, Mag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vigor_fills_one_report() {
        let b = vigor_color(Color(1, 2, 3));
        assert_eq!(b.len(), 65);
        assert_eq!(b[49..52], [1, 2, 3]);
    }

    #[test]
    fn mag_indexes_run_on() {
        let [a, b] = mag_reports(&[Color(9, 9, 9); 52]);
        assert_eq!(a[..9], [0, 0x0C, 0, 40, 0, 0, 9, 9, 9]);
        assert_eq!(b[..9], [0, 0x0C, 0, 12, 0, 40, 9, 9, 9]);
        assert_eq!(b.len(), 5 + 48);
    }
}
