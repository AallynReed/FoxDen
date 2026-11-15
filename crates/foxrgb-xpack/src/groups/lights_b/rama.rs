//! Rama Works M65-B keyboard (protocol learned from SignalRGB's Rama_M65B.js).

use super::layouts::RAMA_POS;
use super::{KEYBOARD, boxed, det, placed};
use crate::hid::{Io, Proto, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const LEN: usize = 64;
const PER_REPORT: usize = 9;
/// Reports start at offsets 0..=72 in steps of nine, so only the first 81 LEDs are driven.
const DRIVEN: usize = 81;

pub static DETECTORS: &[Detector] = &[det(
    "Rama Works M65-B",
    0x5241,
    0x065B,
    None,
    Some(0xFF60),
    Some(0x61),
    open,
)];

/// `sendColors`: nine LEDs from `offset`, then `Apply` (07 42).
pub fn reports(colors: &[Color]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = (0..DRIVEN)
        .step_by(PER_REPORT)
        .map(|offset| {
            let mut b = vec![0x00, 0x07, 0x41, offset as u8];
            for i in offset..offset + PER_REPORT {
                b.extend(rgb(colors.get(i).copied().unwrap_or(Color(0, 0, 0))));
            }
            b
        })
        .collect();
    out.push(vec![0x00, 0x07, 0x42]);
    out
}

struct M65;

impl Proto for M65 {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        reports(c).iter().try_for_each(|p| io.write(p, LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        "rama-m65b",
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        [17, 6],
        &RAMA_POS[..DRIVEN],
    );
    boxed(api, info, "rama m65-b", dev, M65)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_reports_and_apply() {
        let r = reports(&[Color(1, 2, 3); DRIVEN]);
        assert_eq!(r.len(), 10);
        assert_eq!(r[8][..7], [0, 7, 0x41, 72, 1, 2, 3]);
        assert_eq!(r[9], [0, 7, 0x42]);
        assert!(RAMA_POS.len() >= DRIVEN);
    }
}
