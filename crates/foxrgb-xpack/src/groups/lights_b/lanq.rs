//! LANQ PCDOCK light strip (protocol learned from SignalRGB's PCDOCK_Light_strip.js).

use super::{STRIP, boxed, det, row};
use crate::hid::{Io, Proto, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "lanq-pcdock";
/// SignalRGB sizes the strip from its component list; FoxRGB drives a fixed 60.
const LEDS: usize = 60;
const PER_PACKET: usize = 20;
const LEN: usize = 65;
/// Hardware effect defaults stored on first render (mode, speed, brightness, colour, count, direction).
const SETTINGS: &[&[u8]] = &[
    &[0x01, 0x01],
    &[0x02, 0x05],
    &[0x03, 0x64],
    &[0x04, 0x80, 0x00, 0x80],
    &[0x08, 0x00, 0x20, 0x03],
    &[0x09, 0x00, 0x00],
];

pub static DETECTORS: &[Detector] = &[det(
    "LANQ PCDOCK Light Strip",
    0x1C01,
    0x1001,
    Some(2),
    None,
    None,
    open,
)];

/// Report 0 carrying one 64-byte packet.
fn wrap(packet: &[u8]) -> Vec<u8> {
    let mut b = vec![0x00];
    b.extend_from_slice(packet);
    b
}

/// `SendChannel`/`BuildHIDPacket`: packet number from 1, end flag, packet total, channel 1, RGB.
pub fn packets(colors: &[Color], shutdown: bool) -> Vec<Vec<u8>> {
    let total = colors.len().div_ceil(PER_PACKET) as u8;
    colors
        .chunks(PER_PACKET)
        .enumerate()
        .map(|(i, chunk)| {
            let mut p = vec![i as u8 + 1, shutdown as u8, total, 0x01];
            p.extend(chunk.iter().flat_map(|&c| rgb(c)));
            wrap(&p)
        })
        .collect()
}

struct Pcdock;

impl Proto for Pcdock {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&wrap(&[0x00, 0x01]), LEN)?;
        for s in SETTINGS {
            let mut p = vec![0xFF, 0xFF];
            p.extend_from_slice(s);
            io.write(&wrap(&p), LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        packets(c, false).iter().try_for_each(|p| io.write(p, LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        for p in packets(&[Color(0, 0, 0); LEDS], true) {
            let _ = io.write(&p, LEN);
        }
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = row(info, FAMILY, d.name, DeviceKind::Strip, STRIP, LEDS);
    boxed(api, info, "lanq pcdock", dev, Pcdock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_packets() {
        let p = packets(&[Color(1, 2, 3); 21], false);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..8], [0, 1, 0, 2, 1, 1, 2, 3]);
        assert_eq!(p[1][..8], [0, 2, 0, 2, 1, 1, 2, 3]);
        assert_eq!(packets(&[Color(0, 0, 0)], true)[0][2], 1);
    }
}
