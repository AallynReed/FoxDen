//! Aula Win HE, BYTech (Aula Hero HE) and Dareu Cool68 keyboards (protocol learned from SignalRGB's Aula_Win_HE_Controller.js, BYTech_Keyboard_Controller.js and Dareu_Cool68_Keyboard.js).

use super::super::common::{KEYBOARD_RECT, full, keyed};
use super::layouts::*;
use crate::hid::{Driver, Io, Proto, open_io};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const LEN: usize = 64;
/// Aula frames carry 18 LEDs (54 bytes) per packet.
const AULA_CHUNK: usize = 54;
/// Colour-group frames carry 56 data bytes per packet.
const GROUP_CHUNK: usize = 56;
/// The firmware takes at most 40 colour groups; more are merged.
const MAX_GROUPS: usize = 40;
const MERGE_DISTANCE: u32 = 15;

/// Aula: index-mapped RGB in packets `[1, 9, 0, n hi, n lo, len, data]`.
pub fn aula_frame(leds: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    let top = leds.iter().copied().max().map_or(0, |m| m as usize + 1);
    let mut b = vec![0u8; top * 3];
    for (i, c) in leds.iter().zip(colors) {
        let at = *i as usize * 3;
        b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b.chunks(AULA_CHUNK)
        .enumerate()
        .map(|(n, ch)| {
            let mut p = vec![0x01, 0x09, 0x00, (n >> 8) as u8, n as u8, ch.len() as u8];
            p.extend_from_slice(ch);
            p
        })
        .collect()
}

struct Aula(&'static [u8]);

impl Proto for Aula {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for p in [
            [0x01, 0x07, 0x01],
            [0x01, 0x08, 0x01],
            [0x01, 0x09, 0x20],
            [0x01, 0x09, 0x80],
        ] {
            io.write(&p, LEN)?;
            io.sleep(5);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in aula_frame(self.0, colors) {
            io.write(&p, LEN)?;
            io.sleep(5);
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&[0x01, 0x09, 0x81], LEN);
    }
}

fn open_aula(api: &HidApi, info: &Info, _d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (name, size, layout) = if info.product.contains("60") {
        ("Aula Win60 HE", [14, 5], &AULA_WIN60)
    } else {
        ("Aula Win68 HE", [15, 5], &AULA_WIN68)
    };
    let dev = keyed(
        &info.device_id("aula-he"),
        name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        size,
        layout.1,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "aula-he", Aula(layout.0)).boxed())
}

/// `0xFF - (sum & 0xFF)` written at byte 63.
fn sealed(mut p: Vec<u8>) -> Vec<u8> {
    let sum = p.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    p.resize(64, 0);
    p[63] = 0xFF - sum;
    p
}

/// LEDs grouped by colour in first-seen order; over 40 groups, near colours are merged.
pub fn groups(leds: &[u8], colors: &[Color]) -> Vec<(Color, Vec<u8>)> {
    let mut g: Vec<(Color, Vec<u8>)> = Vec::new();
    for (i, c) in leds.iter().zip(colors) {
        match g.iter_mut().find(|(k, _)| k == c) {
            Some((_, v)) => v.push(*i),
            None => g.push((*c, vec![*i])),
        }
    }
    if g.len() <= MAX_GROUPS {
        return g;
    }
    let dist = |a: Color, b: Color| {
        a.0.abs_diff(b.0) as u32 + a.1.abs_diff(b.1) as u32 + a.2.abs_diff(b.2) as u32
    };
    let mut used = vec![false; g.len()];
    let mut out = Vec::new();
    for i in 0..g.len() {
        if used[i] {
            continue;
        }
        let mut merged = g[i].1.clone();
        for j in i + 1..g.len() {
            if !used[j] && dist(g[i].0, g[j].0) <= MERGE_DISTANCE {
                merged.extend_from_slice(&g[j].1);
                used[j] = true;
            }
        }
        used[i] = true;
        out.push((g[i].0, merged));
        if out.len() >= MAX_GROUPS {
            break;
        }
    }
    out
}

/// Colour-group frame: `[r, g, b, n, ids...]` per group, split into `[9, 8, 1, 0, total, i, len]` packets.
pub fn group_frame(leds: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    let flat: Vec<u8> = groups(leds, colors)
        .into_iter()
        .flat_map(|(c, ids)| {
            let mut v = vec![c.0, c.1, c.2, ids.len() as u8];
            v.extend(ids);
            v
        })
        .collect();
    let total = flat.len().div_ceil(GROUP_CHUNK);
    flat.chunks(GROUP_CHUNK)
        .enumerate()
        .map(|(i, ch)| {
            let mut p = vec![0x09, 0x08, 0x01, 0x00, total as u8, i as u8, ch.len() as u8];
            p.extend_from_slice(ch);
            sealed(p)
        })
        .collect()
}

/// Dareu start-up sequence, as its own software sends it, ending with the light bar off.
const DAREU_INIT: &[&[u8]] = &[
    &[0x09, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06],
    &[0x09, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06],
    &[0x09, 0x90, 0x00, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x82, 0x09, 0x00, 0x01, 0x00, 0x00],
    &[0x09, 0x92, 0x00, 0x00, 0x01, 0x00, 0x00],
    &[0x09, 0x9A, 0x00, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x9A, 0x01, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x9A, 0x02, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x82, 0x02, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x82, 0x02, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x82, 0x02, 0x00, 0x01, 0x00, 0x01],
    &[0x09, 0x84, 0x01, 0x00, 0x01],
    &[
        0x09, 0x84, 0x01, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[0x09, 0x84, 0x01, 0x00, 0x01],
    &[
        0x09, 0x84, 0x01, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[0x09, 0x84, 0x01, 0x00, 0x01],
    &[
        0x09, 0x84, 0x01, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[
        0x09, 0x04, 0x01, 0x00, 0x01, 0x00, 0x07, 0x01, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[
        0x09, 0x04, 0x01, 0x00, 0x01, 0x00, 0x07, 0x01, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[
        0x09, 0x04, 0x01, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[
        0x09, 0x04, 0x01, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x14, 0x02, 0x00,
    ],
    &[0x09, 0x84, 0x06, 0x00, 0x01],
    &[
        0x09, 0x04, 0x06, 0x00, 0x01, 0x00, 0x07, 0x03, 0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x00,
    ],
    &[
        0x09, 0x04, 0x06, 0x00, 0x01, 0x00, 0x07, 0x03, 0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x00,
    ],
];

struct Grouped {
    leds: &'static [u8],
    init: &'static [&'static [u8]],
}

impl Proto for Grouped {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for p in self.init {
            io.write(&sealed(p.to_vec()), LEN)?;
            io.sleep(1);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in group_frame(self.leds, colors) {
            io.write(&p, LEN)?;
            io.sleep(1);
        }
        Ok(())
    }
}

fn open_bytech(api: &HidApi, info: &Info, _d: &Detector) -> Result<Box<dyn Backend>, String> {
    if info.product.contains("Dongle") {
        return Err("the BYTech wireless dongle does not take lighting".into());
    }
    let (name, layout) = if info.product.contains("68") {
        ("AULA Hero 68 HE", &HERO68)
    } else {
        ("AULA Hero 84 HE", &HERO84)
    };
    let dev = keyed(
        &info.device_id("bytech"),
        name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [16, 6],
        layout.1,
    );
    let p = Grouped {
        leds: layout.0,
        init: &[],
    };
    Ok(Driver::new(open_io(api, info)?, dev, "bytech", p).boxed())
}

fn open_dareu(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("dareu"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [15, 5],
        DAREU_CELLS,
    );
    let p = Grouped {
        leds: DAREU,
        init: DAREU_INIT,
    };
    Ok(Driver::new(open_io(api, info)?, dev, "dareu", p).boxed())
}

pub static DETECTORS: &[Detector] = &[
    full("Aula Win HE", 0x2E3C, 0xC365, 2, 0xFF1B, 0x0091, open_aula),
    full(
        "AULA Hero HE",
        0x372E,
        0x103E,
        2,
        0xFF60,
        0x0061,
        open_bytech,
    ),
    full(
        "Dareu Cool68",
        0x372E,
        0x1083,
        2,
        0xFF60,
        0x0061,
        open_dareu,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aula_chunks() {
        let p = aula_frame(&[0, 20], &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..9], [1, 9, 0, 0, 0, 54, 1, 2, 3]);
        assert_eq!(p[1][..9], [1, 9, 0, 0, 1, 9, 0, 0, 0]);
        assert_eq!(p[1][12..15], [4, 5, 6]);
    }

    #[test]
    fn grouped_frame_and_checksum() {
        let p = group_frame(
            &[5, 6, 7],
            &[Color(1, 1, 1), Color(2, 2, 2), Color(1, 1, 1)],
        );
        assert_eq!(p.len(), 1);
        assert_eq!(
            p[0][..16],
            [9, 8, 1, 0, 1, 0, 11, 1, 1, 1, 2, 5, 7, 2, 2, 2]
        );
        let sum = p[0][..63].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        assert_eq!(p[0][63], 0xFF - sum);
    }

    #[test]
    fn merges_beyond_forty_groups() {
        let leds: Vec<u8> = (0..60).collect();
        let colors: Vec<Color> = (0..60).map(|i| Color(i as u8 * 4, 0, 0)).collect();
        let g = groups(&leds, &colors);
        assert!(g.len() <= MAX_GROUPS);
        assert_eq!(g[0].1, vec![0, 1, 2, 3]);
    }
}
