//! EVision-based keyboards (VGN, Redragon, Kreo, WOBKEY, EvoFox, M4G) (protocol learned from SignalRGB's EVision_Keyboard_Controller.js).

use super::super::common::{KEYBOARD_RECT, keyed};
use super::layouts::EVISION;
use crate::hid::{Driver, Io, Proto, open_sibling};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info, Open};
use hidapi::HidApi;

const VID: u16 = 0x320F;
const PAGE: u16 = 0xFF1C;
const USAGE: u16 = 0x0092;
const LEN: usize = 64;
/// Data bytes per packet (the plugin's normal frame-rate mode).
const CHUNK: usize = 56;

/// Packet checksum: data sum plus an offset that restarts after the fifth packet.
pub fn checksum(data: &[u8], index: usize) -> u16 {
    let sum: usize = data.iter().map(|b| *b as usize).sum();
    let extra = if index >= 5 {
        (index - 5) * CHUNK + 99
    } else {
        index * CHUNK + 74
    };
    (sum + extra) as u16
}

/// Index-mapped RGB in packets `[4, sum lo, sum hi, 0x12, 56, offset lo, offset hi, 0, data]`.
pub fn frame(leds: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    let top = leds.iter().copied().max().map_or(0, |m| m as usize + 1);
    let mut b = vec![0u8; top * 3];
    for (i, c) in leds.iter().zip(colors) {
        let at = *i as usize * 3;
        b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b.chunks(CHUNK)
        .enumerate()
        .map(|(i, ch)| {
            let [lo, hi] = checksum(ch, i).to_le_bytes();
            let [olo, ohi] = ((i * CHUNK) as u16).to_le_bytes();
            let mut p = vec![0x04, lo, hi, 0x12, CHUNK as u8, olo, ohi, 0x00];
            p.extend_from_slice(ch);
            p
        })
        .collect()
}

struct Evision(&'static [u8]);

impl Proto for Evision {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in frame(self.0, colors) {
            io.write(&p, LEN)?;
            io.sleep(1);
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, _d: &Detector) -> Result<Box<dyn Backend>, String> {
    let m = EVISION
        .iter()
        .find(|m| m.0 == info.product)
        .ok_or_else(|| format!("EVision model \"{}\" is not in the library", info.product))?;
    let io =
        m.5.iter()
            .find_map(|i| open_sibling(api, info, Some(*i), Some(PAGE), Some(USAGE)).ok())
            .ok_or("EVision lighting collection not found")?;
    let dev = keyed(
        &info.device_id("evision"),
        m.1,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        m.2,
        m.4,
    );
    Ok(Driver::new(Box::new(io), dev, "evision", Evision(m.3)).boxed())
}

const fn det(pid: u16, open: Open) -> Detector {
    Detector {
        name: "EVision Keyboard",
        vid: VID,
        pid,
        interface: None,
        usage_page: Some(PAGE),
        usage: Some(USAGE),
        open,
    }
}

pub static DETECTORS: &[Detector] = &[det(0x5055, open), det(0x5088, open)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_and_checksums() {
        let p = frame(&[0, 20], &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..11], [4, 74 + 6, 0, 0x12, 56, 0, 0, 0, 1, 2, 3]);
        assert_eq!(p[1][5..7], [56, 0]);
        assert_eq!(p[1][1], (15 + 56 + 74) as u8);
        assert_eq!(checksum(&[], 5), 99);
        assert_eq!(checksum(&[1], 6), 1 + 56 + 99);
    }

    #[test]
    fn layouts_line_up() {
        for m in EVISION {
            assert_eq!(m.3.len(), m.4.len(), "{}", m.0);
        }
    }
}
