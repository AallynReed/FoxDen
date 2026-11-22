//! Madlions MAD 68 HE keyboard.

use super::layouts::{MAD68_MAP, MAD68_POS};
use super::{KEYBOARD, boxed, by_index, det, placed};
use crate::hid::{Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "madlions";
const BANKS: usize = 5;
/// Fifteen LEDs (45 bytes) per bank, sent as 24 and 21 bytes.
const BANK_BYTES: usize = 45;
const LEN: usize = 33;
const LATCH: [u8; 10] = [0x00, 0x07, 0x41, 0x01, 0x00, 0x3A, 0xFF, 0x00, 0x00, 0xD2];

const fn mad(pid: u16) -> Detector {
    det(
        "Madlions MAD 68 HE",
        0x373B,
        pid,
        Some(1),
        Some(0xFF60),
        Some(0x61),
        open,
    )
}

pub static DETECTORS: &[Detector] = &[mad(0x105C), mad(0x1058)];

/// One bank half: offset 0 (24 bytes) or 8 (21 bytes) of `bank`.
fn half(bank: u8, offset: u8, bytes: &[u8]) -> Vec<u8> {
    let mut p = vec![0x00, 0x07, 0x42, bank, offset, 0x08];
    p.extend_from_slice(bytes);
    p.extend([0, 0, 0]);
    p
}

/// `sendColors`: per bank the first half, the 0x41 report after bank 0, then the second half.
pub fn reports(colors: &[Color]) -> Vec<Vec<u8>> {
    let data: Vec<u8> = by_index(MAD68_MAP, colors, BANKS * 15)
        .iter()
        .flat_map(|&c| rgb(c))
        .collect();
    let mut out = Vec::new();
    for bank in 0..BANKS {
        let at = bank * BANK_BYTES;
        out.push(half(bank as u8, 0x00, &data[at..at + 24]));
        if bank == 0 {
            out.push(LATCH.to_vec());
        }
        out.push(half(bank as u8, 0x08, &data[at + 24..at + BANK_BYTES]));
    }
    out
}

struct Mad68;

impl Proto for Mad68 {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        reports(c).iter().try_for_each(|p| io.write(p, LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &vec![Color(0, 0, 0); MAD68_MAP.len()]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        FAMILY,
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        [15, 5],
        MAD68_POS,
    );
    boxed(api, info, "madlions", dev, Mad68)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_reports() {
        let r = reports(&vec![Color(1, 2, 3); MAD68_MAP.len()]);
        assert_eq!(r.len(), 11);
        assert_eq!(r[0][..9], [0, 7, 0x42, 0, 0, 8, 1, 2, 3]);
        assert_eq!(r[1], LATCH);
        assert_eq!(r[2][..6], [0, 7, 0x42, 0, 8, 8]);
        assert_eq!(r[0].len(), 33);
        assert_eq!(MAD68_POS.len(), MAD68_MAP.len());
    }
}
