//! PrismRGB Prism Mini (one ARGB channel) and Prism S (ATX and triple 8-pin GPU strimers)
//! (protocol learned from SignalRGB's PrismRGB/Prism_Mini.js and PrismRGB/Prism_S.js).

use super::layouts::{PRISM_ATX_POS, PRISM_GPU3_POS};
use super::{STRIP, boxed, det, placed, row};
use crate::hid::{Io, Proto, drain, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x16D0;
const LEN: usize = 65;

pub static DETECTORS: &[Detector] = &[
    det("Prism Mini", VID, 0x1407, Some(2), None, None, open_mini),
    det("Prism S", VID, 0x1294, Some(2), None, None, open_s),
];

/// SignalRGB sizes the channel from its components; FoxRGB drives a fixed 60.
const MINI_LEDS: usize = 60;
const MINI_PER_PACKET: usize = 20;
/// Low Power Saver (on by default): an LED whose R+G+B exceeds this is scaled down to it.
const POWER_CAP: u32 = 175;

fn capped(c: Color) -> Color {
    let total = c.0 as u32 + c.1 as u32 + c.2 as u32;
    if total <= POWER_CAP {
        return c;
    }
    let f = |v: u8| (v as u32 * POWER_CAP / total) as u8;
    Color(f(c.0), f(c.1), f(c.2))
}

/// `SendChannel`: packet number from 1, packet total, 0, 0xAA, twenty capped colours.
pub fn mini_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let total = colors.len().div_ceil(MINI_PER_PACKET) as u8;
    colors
        .chunks(MINI_PER_PACKET)
        .enumerate()
        .map(|(i, chunk)| {
            let mut p = vec![0x00, i as u8 + 1, total, 0x00, 0xAA];
            p.extend(chunk.iter().flat_map(|&c| rgb(capped(c))));
            p
        })
        .collect()
}

struct Mini;

impl Proto for Mini {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[0x00, 0x00, 0x00, 0x00, 0xCC], LEN)?;
        io.read(4, 10)?;
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        drain(io, 4);
        mini_packets(c).iter().try_for_each(|p| io.write(p, LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); MINI_LEDS]);
    }
}

fn open_mini(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = row(
        info,
        "prism-mini",
        d.name,
        DeviceKind::Strip,
        STRIP,
        MINI_LEDS,
    );
    boxed(api, info, "prism mini", dev, Mini)
}

const SEGMENT: usize = 63;
/// The plugin halves every strimer colour.
fn halved(c: Color) -> [u8; 3] {
    [c.0 / 2, c.1 / 2, c.2 / 2]
}

/// `Render` for ATX + triple 8-pin: id-prefixed 63-byte segments, sent 64 bytes per report.
pub fn s_reports(colors: &[Color]) -> Vec<Vec<u8>> {
    let atx_n = PRISM_ATX_POS.len();
    let bytes = |range: std::ops::Range<usize>| -> Vec<u8> {
        range
            .flat_map(|i| halved(colors.get(i).copied().unwrap_or(Color(0, 0, 0))))
            .collect()
    };
    let atx = bytes(0..atx_n);
    let gpu = bytes(atx_n..atx_n + PRISM_GPU3_POS.len());
    let mut stream = Vec::new();
    let mut rest = &atx[..];
    for id in [0u8, 1, 2, 3, 4, 5] {
        let take = rest.len().min(SEGMENT);
        stream.push(id);
        stream.extend_from_slice(&rest[..take]);
        rest = &rest[take..];
    }
    stream.extend_from_slice(&gpu[..18]);
    let mut rest = &gpu[18..];
    for id in 6u8..14 {
        let take = rest.len().min(SEGMENT);
        stream.push(id);
        stream.extend_from_slice(&rest[..take]);
        rest = &rest[take..];
    }
    stream
        .chunks(64)
        .map(|chunk| {
            let mut p = vec![0x00];
            p.extend_from_slice(chunk);
            p
        })
        .collect()
}

/// `Save_Settings`: the colour kept for shutdown, and 0 for the triple 8-pin cable.
fn settings(c: Color) -> [u8; 7] {
    [0x00, 0xFE, 0x01, c.0, c.1, c.2, 0x00]
}

struct PrismS;

impl Proto for PrismS {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&settings(Color(0, 0, 0)), LEN)?;
        io.sleep(50);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        s_reports(c).iter().try_for_each(|p| io.write(p, LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&settings(Color(0, 0, 0)), LEN);
    }
}

fn open_s(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let pos: Vec<[u8; 2]> = PRISM_ATX_POS
        .iter()
        .copied()
        .chain(PRISM_GPU3_POS.iter().map(|p| [p[0] + 20, p[1]]))
        .collect();
    let dev = placed(
        info,
        "prism-s",
        d.name,
        DeviceKind::Strip,
        STRIP,
        [47, 6],
        &pos,
    );
    boxed(api, info, "prism s", dev, PrismS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_cap() {
        assert_eq!(capped(Color(255, 255, 255)), Color(58, 58, 58));
        assert_eq!(capped(Color(100, 50, 25)), Color(100, 50, 25));
        let p = mini_packets(&[Color(10, 20, 30); 21]);
        assert_eq!(p[1][..8], [0, 2, 2, 0, 0xAA, 10, 20, 30]);
    }

    #[test]
    fn strimer_stream() {
        let n = PRISM_ATX_POS.len() + PRISM_GPU3_POS.len();
        assert_eq!((PRISM_ATX_POS.len(), PRISM_GPU3_POS.len()), (120, 162));
        let r = s_reports(&vec![Color(200, 100, 50); n]);
        assert_eq!(r.len(), 14);
        assert_eq!(r[0][..5], [0, 0, 100, 50, 25]);
        assert_eq!(r[1][1], 1);
        assert_eq!(r[5][1], 5);
        assert_eq!(r[6][1], 6);
        assert_eq!(r[13].len(), 1 + 28);
    }
}
