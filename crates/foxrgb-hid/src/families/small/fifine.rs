//! FIFINE HT6 wireless headset lights, through its dongle.

use std::time::{Duration, Instant};

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "fifine";
const ID: u8 = 0x55;
/// Every colour change is saved to flash, so changes land at most this often.
const GAP: Duration = Duration::from_millis(1000);
const TABLE: usize = 8;

fn report(head: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..head.len()].copy_from_slice(head);
    b
}

/// Enable, stop, static mode, colour table, save, start; black only disables the lights.
pub fn sequence(c: Color) -> Vec<Vec<u8>> {
    if c == Color::BLACK {
        return vec![report(&[ID, 0x0C, 0x00])];
    }
    let mut v = vec![
        report(&[ID, 0x0C, 0x01]),
        report(&[ID, 0x02, 0x00]),
        report(&[ID, 0x03, 0x00, TABLE as u8, 0x00, 0x0A, 0x00]),
    ];
    for base in (0..TABLE).step_by(4) {
        let mut b = report(&[ID, 0x04, base as u8, 0x00]);
        for i in 0..4 {
            let keep = 128 - (base + i) as u16;
            let fade = |x: u8| (x as u16 * keep / 128) as u8;
            b[4 + i * 3..7 + i * 3].copy_from_slice(&[fade(c.0), fade(c.1), fade(c.2)]);
        }
        v.push(b);
    }
    v.push(report(&[ID, 0x05, 0x00]));
    v.push(report(&[ID, 0x01, 0x00]));
    v
}

struct Ht6 {
    written: Option<Color>,
    at: Option<Instant>,
}

impl Proto for Ht6 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let c = colors[0];
        if self.written == Some(c) || self.at.is_some_and(|t| t.elapsed() < GAP) {
            return Ok(());
        }
        for p in sequence(c) {
            io.write(&p)?;
            io.sleep(10);
        }
        self.written = Some(c);
        self.at = Some(Instant::now());
        Ok(())
    }

    fn keepalive_ms(&self) -> Option<u64> {
        Some(GAP.as_millis() as u64)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(
        api,
        info,
        FAMILY,
        dev,
        Ht6 {
            written: None,
            at: None,
        },
    )
}

pub static DETECTORS: &[Detector] = &[det("FIFINE HT6", 0x3142, 0x00C3, ANY, 0xFF90, 0x01, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn static_colour_sequence() {
        let s = sequence(Color(128, 64, 0));
        assert_eq!(s.len(), 7);
        assert_eq!(s[0][..3], [0x55, 0x0C, 0x01]);
        assert_eq!(s[1][..3], [0x55, 0x02, 0x00]);
        assert_eq!(s[2][..7], [0x55, 0x03, 0x00, 0x08, 0x00, 0x0A, 0x00]);
        assert_eq!(s[3][..10], [0x55, 0x04, 0, 0, 128, 64, 0, 127, 63, 0]);
        assert_eq!(s[4][..4], [0x55, 0x04, 4, 0]);
        assert_eq!(s[4][4..7], [124, 62, 0]);
        assert_eq!(s[4][13..16], [121, 60, 0]);
        assert_eq!(s[5][..3], [0x55, 0x05, 0x00]);
        assert_eq!(s[6][..3], [0x55, 0x01, 0x00]);
        assert!(s.iter().all(|p| p.len() == 64));
        assert_eq!(sequence(Color::BLACK), vec![pad(&[0x55, 0x0C, 0x00], 64)]);
    }

    #[test]
    fn changes_wait_for_the_gap() {
        let mut h = Ht6 {
            written: None,
            at: None,
        };
        let mut rec = Rec::default();
        h.frame(&mut rec, &[Color(1, 1, 1)]).unwrap();
        assert_eq!(rec.take().len(), 14);
        h.frame(&mut rec, &[Color(2, 2, 2)]).unwrap();
        assert!(rec.take().is_empty());
        h.at = Some(Instant::now() - GAP);
        h.frame(&mut rec, &[Color(2, 2, 2)]).unwrap();
        assert_eq!(rec.take().len(), 14);
    }
}
