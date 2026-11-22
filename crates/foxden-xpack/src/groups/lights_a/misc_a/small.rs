//! A4Tech, Attack Shark, Dark Project, Drevo, Gigabyte Aorus AIO and ASRock monitor.

use super::super::common::{KEYBOARD_RECT, MOUSE_RECT, SMALL_RECT, full, keyed, placed};
use super::layouts::*;
use crate::hid::{Driver, Io, Proto, drain, on_i, open_io, read_until};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info, Open};
use hidapi::HidApi;

/// Colours written at `index * 3` of a buffer of `slots` LEDs.
fn indexed(slots: usize, indexes: &[u8], colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; slots * 3];
    for (i, c) in indexes.iter().zip(colors) {
        let at = *i as usize * 3;
        if at + 3 <= b.len() {
            b[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
        }
    }
    b
}

type Build = Box<dyn Fn(&[Color]) -> Vec<(Vec<u8>, u64)> + Send>;

/// Sends fixed packets at start and a built list per frame, with an optional pause after each.
struct Simple {
    init: Vec<(Vec<u8>, usize)>,
    feature: bool,
    len: usize,
    pause: u64,
    build: Build,
}

impl Simple {
    fn send(&self, io: &mut dyn Io, p: &[u8], len: usize) -> Result<(), String> {
        if self.feature {
            io.feature(p, len)
        } else {
            io.write(p, len)
        }
    }
}

impl Proto for Simple {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for (p, len) in &self.init {
            self.send(io, p, *len)?;
            io.sleep(self.pause);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (p, wait) in (self.build)(colors) {
            self.send(io, &p, self.len)?;
            io.sleep(wait);
        }
        Ok(())
    }
}

fn boxed(
    api: &HidApi,
    info: &Info,
    dev: foxden_core::Device,
    family: &'static str,
    p: Simple,
) -> Result<Box<dyn Backend>, String> {
    Ok(Driver::new(open_io(api, info)?, dev, family, p).boxed())
}

const A4TECH_LEDS: [u8; 8] = [1, 0, 2, 3, 4, 5, 6, 7];
const A4TECH_CELLS: [[u8; 2]; 8] = [
    [0, 0],
    [1, 0],
    [2, 0],
    [0, 2],
    [2, 2],
    [0, 4],
    [1, 4],
    [2, 4],
];

/// Bloody A70x frame: feature report 7, `[7, 3, 6, 2, 0, 0, 0, 0, rgb by LED id]`.
pub fn a4tech_frame(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x07, 0x03, 0x06, 0x02, 0x00, 0x00, 0x00, 0x00];
    p.extend(indexed(8, &A4TECH_LEDS, colors));
    p
}

fn open_a4tech(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("a4tech"),
        d.name,
        DeviceKind::Mouse,
        MOUSE_RECT,
        [3, 5],
        &A4TECH_CELLS,
    );
    let init = vec![
        (vec![0x07, 0x03, 0x06, 0x01, 0, 0, 0, 0], 65),
        (vec![0x07, 0x03, 0x06, 0x00, 0, 0, 0, 0, 0x01], 65),
    ];
    let p = Simple {
        init,
        feature: true,
        len: 65,
        pause: 0,
        build: Box::new(|c| vec![(a4tech_frame(c), 0)]),
    };
    boxed(api, info, dev, "a4tech", p)
}

/// Attack Shark: feature `[0, 7, 1, 4, 4, 7, r, g, b, 255 - sum]`.
pub fn attackshark_frame(c: Color) -> Vec<u8> {
    let mut p = vec![0x00, 0x07, 0x01, 0x04, 0x04, 0x07, c.0, c.1, c.2];
    let sum = p[1..9].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    p.push(255u8.wrapping_sub(sum));
    p
}

fn open_attackshark(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("attackshark"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [1, 1],
        &[[0, 0]],
    );
    let p = Simple {
        init: vec![],
        feature: true,
        len: 65,
        pause: 0,
        build: Box::new(|c| vec![(attackshark_frame(c[0]), 0)]),
    };
    boxed(api, info, dev, "attackshark", p)
}

/// Dark Project: red and green planes (green at +96) in one report, blue in a second, then commit.
pub fn darkproject_frame(colors: &[Color]) -> Vec<(Vec<u8>, u64)> {
    let mut rg = vec![0u8; 192];
    let mut b = vec![0u8; 96];
    for (i, c) in DARKPROJECT.iter().zip(colors) {
        let i = *i as usize;
        rg[i] = c.0;
        rg[i + 96] = c.1;
        b[i] = c.2;
    }
    let mut p1 = vec![0x07, 0x0A, 0x01, 0x04, 0x01, 0, 0, 0];
    p1.extend(rg);
    let mut p2 = vec![0x07, 0x0A, 0x01, 0x04, 0x02, 0, 0, 0];
    p2.extend(b);
    vec![
        (vec![0x07, 0x82, 0x01], 5),
        (p1, 5),
        (p2, 5),
        (vec![0x07, 0x8B], 75),
    ]
}

fn open_darkproject(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("darkproject"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [18, 6],
        DARKPROJECT_CELLS,
    );
    let p = Simple {
        init: vec![],
        feature: true,
        len: 256,
        pause: 0,
        build: Box::new(darkproject_frame),
    };
    boxed(api, info, dev, "darkproject", p)
}

/// Drevo slots: 133 LEDs, the first in the header report, then 66 reports of two LEDs.
const DREVO_SLOTS: usize = 133;

pub fn drevo_frame(colors: &[Color], footer: u8) -> Vec<(Vec<u8>, u64)> {
    let b = indexed(DREVO_SLOTS, DREVO, colors);
    let mut v = vec![(vec![0x05, 0xF3, 0x00, 0x00, 0x80, b[0], b[1], b[2]], 1)];
    for ch in b[3..].chunks(6) {
        let mut p = vec![0x05, 0x00];
        p.extend_from_slice(ch);
        v.push((p, 1));
    }
    v.push((vec![0x05, 0, 0, 0, 0, 0, 0, footer], 0));
    v
}

fn drevo_init() -> Vec<(Vec<u8>, usize)> {
    let mut v: Vec<(Vec<u8>, usize)> = (0..5)
        .map(|_| (vec![0x05, 0xF4, 0, 0, 0, 0, 0, 0], 8))
        .collect();
    for p in [
        [0x05, 0xFE, 0x00, 0x02, 0x01, 0, 0, 0],
        [0x05, 0xFB, 0, 0, 0, 0, 0, 0],
        [0x05, 0xFA, 0, 0, 0, 0, 0, 0],
        [0x05, 0xFA, 0, 0, 0, 0, 0, 0],
        [0x05, 0xFE, 0x0C, 0x0F, 0, 0, 0, 0],
        [0x05, 0xFE, 0x0C, 0x0F, 0, 0, 0, 0],
        [0x05, 0xFE, 0x0D, 0, 0, 0, 0, 0],
    ] {
        v.push((p.to_vec(), 8));
    }
    // a black frame closed with footer 0x22 finishes the start-up
    v.extend(drevo_frame(&[], 0x22).into_iter().map(|(p, _)| (p, 8)));
    v
}

fn open_drevo(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("drevo"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [22, 8],
        DREVO_CELLS,
    );
    let p = Simple {
        init: drevo_init(),
        feature: true,
        len: 8,
        pause: 1,
        build: Box::new(|c| drevo_frame(c, 0xE2)),
    };
    boxed(api, info, dev, "drevo", p)
}

const AORUS_CELLS: [[u8; 2]; 9] = [
    [4, 6],
    [9, 3],
    [8, 1],
    [7, 3],
    [7, 5],
    [7, 7],
    [8, 9],
    [9, 7],
    [9, 5],
];
const AORUS_STATIC: u8 = 0x01;
const AORUS_SPEED: u8 = 2;

fn aorus_state(part1: u8, part2: u8) -> Vec<u8> {
    vec![
        0x00,
        0xC9,
        AORUS_STATIC,
        100,
        AORUS_SPEED,
        part1,
        part2,
        0,
        0,
    ]
}

/// Static mode frame: logo `0xBC`, then ring LED pairs on `0xB0..0xB3`.
pub fn aorus_frame(colors: &[Color]) -> Vec<(Vec<u8>, u64)> {
    let c = |i: usize| colors[i];
    let mut v = vec![(vec![0x00, 0xBC, c(0).0, c(0).1, c(0).2, 0, 0, 0, 0], 0)];
    for z in 0..4 {
        let (a, b) = (c(1 + z * 2), c(2 + z * 2));
        v.push((
            vec![
                0x00,
                0xB0 + z as u8,
                AORUS_STATIC,
                a.0,
                a.1,
                a.2,
                b.0,
                b.1,
                b.2,
            ],
            0,
        ));
    }
    v
}

fn open_aorus(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("aorus-aio"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [10, 10],
        &AORUS_CELLS,
    );
    let init = vec![
        (aorus_state(0x00, 0x00), 9),
        (aorus_state(0x08, 0x01), 9),
        (vec![0x00, 0xBE, 0, 0, 0, 0, 0, 0, 0xCC], 9),
    ];
    let p = Simple {
        init,
        feature: true,
        len: 9,
        pause: 0,
        build: Box::new(aorus_frame),
    };
    boxed(api, info, dev, "aorus-aio", p)
}

/// ASRock monitor: header `[0, 0x10, 0, 2, 0xE3, 0, 0, count lo, count hi]` with 18 LEDs, then `0xE4` packets of 20.
pub fn asrock_frame(colors: &[Color]) -> Vec<(Vec<u8>, u64)> {
    let data: Vec<u8> = colors.iter().flat_map(|c| [c.0, c.1, c.2]).collect();
    let n = colors.len();
    let first = data.len().min(54);
    let mut head = vec![
        0x00,
        0x10,
        0x00,
        0x02,
        0xE3,
        0x00,
        0x00,
        n as u8,
        (n >> 8) as u8,
    ];
    head.extend_from_slice(&data[..first]);
    let mut v = vec![(head, 0)];
    for ch in data[first..].chunks(60) {
        let mut p = vec![0x00, 0x10, 0x00, 0x02, 0xE4];
        p.extend_from_slice(ch);
        v.push((p, 0));
    }
    v
}

/// Side LED counts from the `0x14 0x0B` reply: left, top, right, bottom (hidapi drops the 0 report id).
fn asrock_counts(api: &HidApi, info: &Info) -> Result<[usize; 4], String> {
    let mut io = open_io(api, info)?;
    drain(io.as_mut(), 64);
    io.write(&[0x00, 0x14, 0x00, 0x0B], 64)?;
    let r = read_until(io.as_mut(), 64, 500, |_| true)?;
    let at = |i: usize| r.get(i - 1).copied().unwrap_or(0) as usize;
    Ok([at(4), at(5), at(6), at(7)])
}

fn open_asrock(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let [l, t, r, b] = asrock_counts(api, info)?;
    let step = |i: usize, n: usize| (i as f32 + 0.5) / n.max(1) as f32;
    let mut leds = Vec::new();
    leds.extend((0..l).map(|i| [0.0, step(i, l)]));
    leds.extend((0..t).map(|i| [step(i, t), 0.0]));
    leds.extend((0..r).map(|i| [1.0, step(i, r)]));
    leds.extend((0..b).map(|i| [step(i, b), 1.0]));
    if leds.is_empty() {
        return Err("ASRock monitor reports no LEDs".into());
    }
    let w = t.max(b).max(1) as u32;
    let h = l.max(r).max(1) as u32;
    let dev = placed(
        &info.device_id("asrock-monitor"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [w, h],
        leds,
    );
    let p = Simple {
        init: vec![],
        feature: false,
        len: 64,
        pause: 0,
        build: Box::new(asrock_frame),
    };
    boxed(api, info, dev, "asrock-monitor", p)
}

const fn page(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: i32,
    page: u16,
    open: Open,
) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: Some(interface),
        usage_page: Some(page),
        usage: None,
        open,
    }
}

pub static DETECTORS: &[Detector] = &[
    full(
        "A4Tech Bloody A70x",
        0x09DA,
        0x1CCA,
        2,
        0xFF32,
        0x04C0,
        open_a4tech,
    ),
    full(
        "A4Tech Bloody A70x",
        0x09DA,
        0x11A1,
        2,
        0xFF32,
        0x04C0,
        open_a4tech,
    ),
    full(
        "A4Tech Bloody A70x",
        0x09DA,
        0x7452,
        2,
        0xFF32,
        0x04C0,
        open_a4tech,
    ),
    page(
        "Attack Shark X68 Pro HE",
        0x3151,
        0x5026,
        2,
        0xFFFF,
        open_attackshark,
    ),
    page(
        "Attack Shark X68 Pro HE",
        0x3151,
        0x502D,
        2,
        0xFFFF,
        open_attackshark,
    ),
    page(
        "DarkProject 87",
        0x342D,
        0xE40F,
        2,
        0xFF01,
        open_darkproject,
    ),
    page(
        "DarkProject 87",
        0x342D,
        0xE410,
        2,
        0xFF01,
        open_darkproject,
    ),
    page(
        "DREVO BladeMaster TE 88K",
        0x1A2C,
        0xB58F,
        0,
        0xFF00,
        open_drevo,
    ),
    full(
        "Gigabyte Aorus Liquid Cooler 360",
        0x1044,
        0x7A46,
        1,
        0xFF00,
        1,
        open_aorus,
    ),
    on_i("ASRock Monitor", 0x26CE, 0x01C4, 0, open_asrock),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_and_planes() {
        assert_eq!(
            attackshark_frame(Color(1, 2, 3))[9],
            (255 - (7 + 1 + 4 + 4 + 7 + 6)) as u8
        );
        let d = darkproject_frame(&[Color(1, 2, 3)]);
        assert_eq!(d.len(), 4);
        assert_eq!(d[1].0[8], 1);
        assert_eq!(d[1].0[8 + 96], 2);
        assert_eq!(d[2].0[8], 3);
        assert_eq!(d[3], (vec![0x07, 0x8B], 75));
    }

    #[test]
    fn drevo_splits_399_bytes() {
        let f = drevo_frame(&[Color(9, 9, 9); 133], 0xE2);
        assert_eq!(f.len(), 1 + 66 + 1);
        assert_eq!(f[67].0[7], 0xE2);
        assert!(f[1..67].iter().all(|(p, _)| p.len() == 8));
    }

    #[test]
    fn a4tech_aorus_asrock() {
        let a = a4tech_frame(&[Color(1, 1, 1), Color(2, 2, 2)]);
        assert_eq!(a[8..14], [2, 2, 2, 1, 1, 1]);
        let g = aorus_frame(&[Color(5, 5, 5); 9]);
        assert_eq!(g[4].0[..3], [0, 0xB3, 1]);
        let r = asrock_frame(&[Color(1, 2, 3); 30]);
        assert_eq!(r[0].0[..9], [0, 0x10, 0, 2, 0xE3, 0, 0, 30, 0]);
        assert_eq!(r[1].0.len(), 5 + 36);
    }
}
