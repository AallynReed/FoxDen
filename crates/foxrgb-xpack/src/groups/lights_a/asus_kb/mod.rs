//! ASUS keyboards, mouse pads and small Aura peripherals (protocol learned from SignalRGB's ASUS_Keyboard.js, ASUS_TUF_K1.js, ASUS_ROG_Claymore_I.js, ASUS_ROG_Claymore_II.js, Asus_ROG_Balteus.js, Asus_ROG_Balteus_Qi.js, Asus_ROG_Throne_Qi_Stand.js, ASUS_ROG_Spotlight.js and ASUS_ROG_Aura_Sync_Logo_Projector.js).

mod claymore_layouts;
mod layouts;

use super::common::{ASUS_RESTORE, ASUS_SAVE, KEYBOARD_RECT, SMALL_RECT, asus_direct, full, keyed};
use crate::hid::{Driver, Io, Proto, drain, on_i, open_io, read_until};
use claymore_layouts::*;
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x0B05;
const LEN: usize = 65;
const PAGE: u16 = 0xFF00;
/// Claymore frames always carry 120 slots; unused ones are 0xFF.
const CLAYMORE_SLOTS: usize = 120;

pub struct KbLayout {
    pub size: [u8; 2],
    pub leds: &'static [u8],
    pub cells: &'static [[u8; 2]],
}

pub const fn kb(name: &'static str, pid: u16) -> Detector {
    full(name, VID, pid, 1, PAGE, 1, open_keyboard)
}

fn zip(leds: &[u8], colors: &[Color]) -> Vec<(u8, Color)> {
    leds.iter()
        .zip(
            colors
                .iter()
                .copied()
                .chain(std::iter::repeat(Color::BLACK)),
        )
        .map(|(i, c)| (*i, c))
        .collect()
}

/// Keyboard frame: 15 LEDs per packet, byte 3 is the count still to send.
pub fn keyboard_packets(leds: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    let total = leds.len();
    zip(leds, colors)
        .chunks(15)
        .enumerate()
        .map(|(n, ch)| asus_direct(0, (total - 15 * n) as u8, ch))
        .collect()
}

/// Claymore frame: 120 slots padded with 0xFF, byte 3 from `count(packet)`.
pub fn claymore_packets(leds: &[u8], colors: &[Color], count: fn(usize) -> u8) -> Vec<Vec<u8>> {
    let mut slots = zip(leds, colors);
    slots.resize(CLAYMORE_SLOTS, (0xFF, Color(0xFF, 0xFF, 0xFF)));
    slots
        .chunks(15)
        .enumerate()
        .map(|(n, ch)| asus_direct(0, count(n), ch))
        .collect()
}

fn restore(io: &mut dyn Io) {
    let _ = io.write(&ASUS_RESTORE, LEN);
    let _ = io.write(&ASUS_SAVE, LEN);
}

type Build = Box<dyn Fn(&[Color]) -> Vec<Vec<u8>> + Send>;

struct Packets {
    build: Build,
    init: &'static [u8],
    restore: bool,
}

impl Proto for Packets {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if !self.init.is_empty() {
            io.write(self.init, LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in (self.build)(colors) {
            io.write(&p, LEN)?;
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        if self.restore {
            restore(io);
        }
    }
}

fn boxed(
    io: Box<dyn Io>,
    dev: foxrgb_core::Device,
    family: &'static str,
    p: Packets,
) -> Result<Box<dyn Backend>, String> {
    Ok(Driver::new(io, dev, family, p).boxed())
}

fn open_keyboard(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let layout = layouts::PIDS
        .iter()
        .find(|(pid, _)| *pid == info.pid)
        .map(|(_, l)| *l)
        .ok_or("unknown ASUS keyboard")?;
    let size = [layout.size[0] as u32, layout.size[1] as u32];
    let dev = keyed(
        &info.device_id("asus-keyboard"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        size,
        layout.cells,
    );
    let leds = layout.leds;
    let p = Packets {
        build: Box::new(move |c| keyboard_packets(leds, c)),
        init: &[],
        restore: true,
    };
    boxed(open_io(api, info)?, dev, "asus-keyboard", p)
}

/// Reads one reply to `query`; hidapi drops the 0 report id, so it is put back to keep plugin offsets.
fn query(io: &mut dyn Io, q: &[u8]) -> Option<Vec<u8>> {
    drain(io, LEN);
    io.write(q, LEN).ok()?;
    let r = read_until(io, LEN, 200, |_| true).ok()?;
    let mut v = vec![0u8];
    v.extend(r);
    Some(v)
}

fn open_claymore(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    let second = info.pid != 0x184D;
    let left = if second {
        // status byte 17: 0xFF or 1 is a right-hand numpad
        query(io.as_mut(), &[0x00, 0x12]).is_some_and(|r| !matches!(r.get(17), Some(0xFF | 0x01)))
    } else {
        query(io.as_mut(), &[0x00, 0x40, 0x60]).is_some_and(|r| r.get(5).is_some_and(|b| *b != 0))
    };
    let (leds, cells, rows): (&'static [u8], &'static [[u8; 2]], u32) = match (second, left) {
        (false, false) => (C1_RIGHT, C1_RIGHT_CELLS, 8),
        (false, true) => (C1_LEFT, C1_LEFT_CELLS, 8),
        (true, false) => (C2_RIGHT, C2_RIGHT_CELLS, 7),
        (true, true) => (C2_LEFT, C2_LEFT_CELLS, 7),
    };
    let dev = keyed(
        &info.device_id("asus-claymore"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [21, rows],
        cells,
    );
    let count: fn(usize) -> u8 = if second {
        |n| 0x90 - 0x0F * n as u8
    } else {
        |_| 0x0F
    };
    let p = Packets {
        build: Box::new(move |c| claymore_packets(leds, c, count)),
        init: if second { &[] } else { &[0x00, 0x41, 0x02] },
        restore: true,
    };
    boxed(io, dev, "asus-claymore", p)
}

/// Spotlight: a static colour command, then the direct colour command.
pub fn spotlight_packets(c: Color) -> Vec<Vec<u8>> {
    let mut direct = vec![0xEC, 0x40, 0x00, 0xEE, 0x01, c.0, c.1, c.2, 0x00, 0x00];
    direct.extend([0x84, 0xF3, 0x12, 0x03, 0x70, 0x12, 0x3B, 0x75, 0x47, 0xA1]);
    vec![vec![0xEC, 0x3B, 0x00, 0xFF, c.0, c.1, c.2], direct]
}

fn open_spotlight(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("asus-spotlight"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [1, 1],
        &[[0, 0]],
    );
    let p = Packets {
        build: Box::new(|c| spotlight_packets(c[0])),
        init: &[],
        restore: false,
    };
    boxed(open_io(api, info)?, dev, "asus-spotlight", p)
}

/// Projector register write: feature report 3, `[3, 2, 0xA1, 0x80, register, value, 0, 0]`.
pub fn projector_reg(register: u8, value: u8) -> [u8; 8] {
    [0x03, 0x02, 0xA1, 0x80, register, value, 0x00, 0x00]
}

/// Colour registers 0x10.. take red, blue, green; register 0xA0 latches them.
pub fn projector_frame(c: Color) -> [[u8; 8]; 4] {
    [
        projector_reg(0x10, c.0),
        projector_reg(0x11, c.2),
        projector_reg(0x12, c.1),
        projector_reg(0xA0, 0x01),
    ]
}

struct Projector;

impl Proto for Projector {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&projector_reg(0x20, 0), 8)?;
        io.feature(&projector_reg(0x30, 0), 8)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for r in projector_frame(colors[0]) {
            io.feature(&r, 8)?;
        }
        Ok(())
    }
}

fn open_projector(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("asus-logo-projector"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [1, 1],
        &[[0, 0]],
    );
    Ok(Driver::new(open_io(api, info)?, dev, "asus-logo-projector", Projector).boxed())
}

static OTHERS: &[Detector] = &[
    full(
        "ASUS ROG Claymore II",
        VID,
        0x196B,
        1,
        PAGE,
        1,
        open_claymore,
    ),
    full(
        "ASUS ROG Claymore II",
        VID,
        0x1934,
        1,
        PAGE,
        1,
        open_claymore,
    ),
    full(
        "ASUS ROG Spotlight",
        VID,
        0x1881,
        0,
        0x00FF,
        1,
        open_spotlight,
    ),
    on_i(
        "ASUS ROG Aura Sync Logo Projector",
        VID,
        0x19B8,
        0,
        open_projector,
    ),
    on_i(
        "ASUS ROG Aura Sync Logo Projector",
        VID,
        0x19B8,
        -1,
        open_projector,
    ),
];

pub fn detectors() -> Vec<&'static Detector> {
    layouts::DETECTORS.iter().chain(OTHERS.iter()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_counts_down_by_fifteen() {
        let leds: Vec<u8> = (0..20).collect();
        let colors = vec![Color(1, 2, 3); 20];
        let p = keyboard_packets(&leds, &colors);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..9], [0, 0xC0, 0x81, 20, 0, 0, 1, 2, 3]);
        assert_eq!(p[1][..9], [0, 0xC0, 0x81, 5, 0, 15, 1, 2, 3]);
        assert_eq!(p[1][25], 0);
        assert!(p.iter().all(|x| x.len() == 65));
    }

    #[test]
    fn claymore_pads_to_120_slots() {
        let p = claymore_packets(&[7], &[Color(9, 8, 7)], |n| 0x90 - 0x0F * n as u8);
        assert_eq!(p.len(), 8);
        assert_eq!(
            p[0][..13],
            [0, 0xC0, 0x81, 0x90, 0, 7, 9, 8, 7, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(p[7][3], 0x90 - 0x0F * 7);
    }

    #[test]
    fn projector_swaps_green_and_blue() {
        let f = projector_frame(Color(1, 2, 3));
        assert_eq!(f[1], [3, 2, 0xA1, 0x80, 0x11, 3, 0, 0]);
        assert_eq!(f[3][4..6], [0xA0, 1]);
        assert_eq!(
            spotlight_packets(Color(1, 2, 3))[0],
            vec![0xEC, 0x3B, 0, 0xFF, 1, 2, 3]
        );
    }

    #[test]
    fn layouts_are_consistent() {
        for (_, l) in layouts::PIDS {
            assert_eq!(l.leds.len(), l.cells.len());
        }
        assert_eq!(layouts::PIDS.len(), layouts::DETECTORS.len());
        assert_eq!(C1_LEFT.len(), C1_LEFT_CELLS.len());
        assert_eq!(C2_RIGHT.len(), C2_RIGHT_CELLS.len());
    }
}
