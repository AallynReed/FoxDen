//! ThermalTake Argent peripherals, DPSG PSU, LEDBOX Pacific and Level 20 desk.

use super::common::{open_with, positioned};
use crate::hid::{Io, Proto, on_i};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x264A;
const LEN: usize = 65;
const BLACK: Color = Color(0, 0, 0);

const ARGENT_INIT: [[u8; 2]; 2] = [[0x41, 0x03], [0x12, 0x22]];
const ARGENT_INIT_LEN: usize = 64;
const ARGENT_HEAD: [u8; 3] = [0x00, 0xC0, 0x01];
const ARGENT_MAX: usize = 15;

/// One Argent device: canvas cells, then (wire slot, LED index) entries sent in `chunks`.
struct Argent {
    pid: u16,
    w: u32,
    h: u32,
    pos: &'static [(u8, u8)],
    entries: &'static [(u8, u8)],
    chunks: &'static [usize],
}

const fn ident<const N: usize>() -> [(u8, u8); N] {
    let mut a = [(0u8, 0u8); N];
    let mut i = 0;
    while i < N {
        a[i] = (i as u8, i as u8);
        i += 1;
    }
    a
}

static IDENT20: [(u8, u8); 20] = ident::<20>();

static BUNGEE_ENTRIES: [(u8, u8); 13] = {
    let mut a = ident::<13>();
    a[11].1 = 10;
    a[12].1 = 10;
    a
};

static E1_SLOTS: [u8; 25] = [
    0x00, 0x08, 0x10, 0x18, 0x01, 0x09, 0x11, 0x19, 0x21, 0x02, 0x0A, 0x12, 0x1A, 0x20, 0x22, 0x03,
    0x0B, 0x13, 0x1B, 0x23, 0x04, 0x0C, 0x14, 0x1C, 0x24,
];

static E1_ENTRIES: [(u8, u8); 25] = {
    let mut a = [(0u8, 0u8); 25];
    let mut i = 0;
    while i < 25 {
        a[i] = (E1_SLOTS[i], i as u8);
        i += 1;
    }
    a
};

static MODELS: [Argent; 4] = [
    Argent {
        pid: 0x8020,
        w: 9,
        h: 9,
        pos: &[
            (4, 8),
            (3, 7),
            (2, 6),
            (1, 5),
            (2, 4),
            (3, 3),
            (4, 4),
            (5, 5),
            (6, 6),
            (5, 7),
            (4, 7),
        ],
        entries: &BUNGEE_ENTRIES,
        chunks: &[13],
    },
    Argent {
        pid: 0x9011,
        w: 11,
        h: 11,
        pos: &[
            (4, 0),
            (2, 0),
            (0, 0),
            (0, 2),
            (0, 4),
            (0, 6),
            (0, 8),
            (0, 10),
            (2, 10),
            (4, 10),
            (6, 10),
            (8, 10),
            (10, 10),
            (10, 8),
            (10, 6),
            (10, 4),
            (10, 2),
            (10, 0),
            (8, 0),
            (6, 0),
        ],
        entries: &IDENT20,
        chunks: &[15, 5],
    },
    Argent {
        pid: 0x8002,
        w: 9,
        h: 11,
        pos: &[
            (3, 0),
            (2, 0),
            (1, 0),
            (0, 2),
            (0, 3),
            (0, 4),
            (1, 5),
            (1, 6),
            (1, 7),
            (2, 8),
            (2, 9),
            (3, 10),
            (5, 10),
            (4, 7),
            (6, 9),
            (6, 8),
            (7, 7),
            (7, 6),
            (7, 5),
            (8, 4),
            (8, 3),
            (8, 2),
            (7, 0),
            (6, 0),
            (5, 0),
        ],
        entries: &E1_ENTRIES,
        chunks: &[15, 10],
    },
    Argent {
        pid: 0x8022,
        w: 7,
        h: 11,
        pos: &[
            (3, 1),
            (3, 2),
            (2, 3),
            (2, 4),
            (1, 5),
            (1, 6),
            (1, 7),
            (1, 8),
            (2, 9),
            (3, 10),
            (4, 10),
            (5, 9),
            (6, 8),
            (6, 7),
            (6, 6),
            (5, 5),
            (5, 4),
            (5, 3),
            (4, 2),
            (4, 1),
        ],
        entries: &IDENT20,
        chunks: &[15, 5],
    },
];

/// Colour packets: `00 C0 01 <count> 00`, then slot, R, G, B per entry.
fn argent_packets(m: &Argent, colors: &[Color]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    for &n in m.chunks {
        let mut p = ARGENT_HEAD.to_vec();
        p.extend_from_slice(&[n.min(ARGENT_MAX) as u8, 0x00]);
        for &(slot, led) in &m.entries[at..at + n] {
            let c = colors.get(led as usize).copied().unwrap_or(BLACK);
            p.extend_from_slice(&[slot, c.0, c.1, c.2]);
        }
        at += n;
        out.push(p);
    }
    out
}

struct ArgentProto(&'static Argent);

impl ArgentProto {
    fn send(&self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in argent_packets(self.0, colors) {
            io.write(&p, LEN)?;
            io.sleep(1);
        }
        Ok(())
    }
}

impl Proto for ArgentProto {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for r in ARGENT_INIT {
            io.write(&r, ARGENT_INIT_LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.send(io, colors)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.send(io, &vec![BLACK; self.0.pos.len()]);
    }
}

fn open_argent(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let m = MODELS
        .iter()
        .find(|m| m.pid == info.pid)
        .ok_or("unknown model")?;
    let dev = positioned(
        &info.device_id("thermaltake-argent"),
        d.name,
        DeviceKind::Other,
        m.w,
        m.h,
        m.pos,
    );
    open_with(api, info, "thermaltake-argent", dev, ArgentProto(m))
}

const PSU_INIT: [u8; 3] = [0x00, 0xFE, 0x31];
const PSU_INIT_LEN: usize = 64;
const PSU_HEAD: [u8; 4] = [0x00, 0x30, 0x42, 0x18];
static PSU_POS: [(u8, u8); 12] = [
    (1, 2),
    (0, 3),
    (1, 4),
    (2, 5),
    (3, 6),
    (4, 5),
    (5, 4),
    (6, 3),
    (5, 2),
    (4, 1),
    (3, 0),
    (2, 0),
];

fn grb(colors: &[Color]) -> Vec<u8> {
    colors.iter().flat_map(|c| [c.1, c.0, c.2]).collect()
}

/// `00 30 42 18`, then 12 LEDs as GRB.
fn psu_packet(colors: &[Color]) -> Vec<u8> {
    let mut p = PSU_HEAD.to_vec();
    p.extend(grb(&colors[..colors.len().min(PSU_POS.len())]));
    p
}

struct Psu;

impl Proto for Psu {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&PSU_INIT, PSU_INIT_LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&psu_packet(colors), LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&psu_packet(&[BLACK; 12]), LEN);
    }
}

fn open_psu(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id("thermaltake-dpsg"),
        d.name,
        DeviceKind::Other,
        7,
        7,
        &PSU_POS,
    );
    open_with(api, info, "thermaltake-dpsg", dev, Psu)
}

const LEDBOX_CHANNELS: usize = 5;
const LEDBOX_LEDS: usize = 36;
const LEDBOX_PER_PACKET: usize = 19;
const LEDBOX_INIT: [[u8; 3]; 2] = [[0x00, 0xFE, 0x33], [0x00, 0xFE, 0x50]];

/// Two packets per channel: `00 32 52 <ch+1> 18 03 <1|2> 00`, then 19 LEDs as GRB.
fn ledbox_packets(channel: u8, colors: &[Color]) -> [Vec<u8>; 2] {
    let data = grb(colors);
    let part = |i: usize| {
        let mut p = vec![0x00, 0x32, 0x52, channel + 1, 0x18, 0x03, i as u8 + 1, 0x00];
        let from = (i * LEDBOX_PER_PACKET * 3).min(data.len());
        let to = (from + LEDBOX_PER_PACKET * 3).min(data.len());
        p.extend_from_slice(&data[from..to]);
        p
    };
    [part(0), part(1)]
}

struct Ledbox;

impl Proto for Ledbox {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&LEDBOX_INIT[0], LEN)?;
        io.read(LEN, 100)?;
        io.feature(&LEDBOX_INIT[1], LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (ch, zone) in colors.chunks(LEDBOX_LEDS).enumerate() {
            for p in ledbox_packets(ch as u8, zone) {
                io.write(&p, LEN)?;
            }
        }
        Ok(())
    }
}

fn open_ledbox(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = Device::grid(
        &info.device_id("thermaltake-ledbox"),
        d.name,
        DeviceKind::Strip,
        super::common::rect(DeviceKind::Strip),
        LEDBOX_LEDS as u32,
        LEDBOX_CHANNELS as u32,
    );
    open_with(api, info, "thermaltake-ledbox", dev, Ledbox)
}

const DESK_LEN: usize = 839;
const DESK_LEDS: usize = 278;
const DESK_INIT: [u8; 3] = [0x00, 0xFE, 0x33];
const DESK_HEAD: [u8; 5] = [0x00, 0x32, 0x52, 0x04, 0x24];

/// Canvas cells of the desk (100 x 43): top edge, right/left sides interleaved, bottom edge.
fn desk_positions() -> Vec<(u8, u8)> {
    let mut v: Vec<(u8, u8)> = (0..100).map(|x| (x, 0)).collect();
    v.push((99, 1));
    for y in 2..=40 {
        v.push((0, y));
        v.push((99, y));
    }
    v.push((99, 41));
    v.extend((1..99).map(|x| (x, 42)));
    v
}

/// Wire slot of each LED, in `desk_positions` order.
fn desk_slots() -> Vec<usize> {
    let mut v: Vec<usize> = (0..100).map(|i| 140 - i).collect();
    v.push(40);
    for k in 0..39 {
        v.push(141 + k);
        v.push(39 - k);
    }
    v.push(0);
    v.extend((0..98).map(|j| 277 - j));
    v
}

/// `00 32 52 04 24`, then 278 LEDs as GRB at their wire slots.
fn desk_packet(slots: &[usize], colors: &[Color]) -> Vec<u8> {
    let mut p = DESK_HEAD.to_vec();
    p.resize(DESK_HEAD.len() + DESK_LEDS * 3, 0);
    for (&slot, c) in slots.iter().zip(colors) {
        let at = DESK_HEAD.len() + slot * 3;
        p[at..at + 3].copy_from_slice(&[c.1, c.0, c.2]);
    }
    p
}

struct Desk(Vec<usize>);

impl Proto for Desk {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&DESK_INIT, DESK_LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&desk_packet(&self.0, colors), DESK_LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&desk_packet(&self.0, &[BLACK; DESK_LEDS]), DESK_LEN);
    }
}

fn open_desk(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id("thermaltake-level20"),
        d.name,
        DeviceKind::Other,
        100,
        43,
        &desk_positions(),
    );
    open_with(api, info, "thermaltake-level20", dev, Desk(desk_slots()))
}

const fn ipu(name: &'static str, pid: u16, open: foxden_hid::registry::Open) -> Detector {
    Detector {
        name,
        vid: VID,
        pid,
        interface: Some(0),
        usage_page: Some(0xFF00),
        usage: Some(0x0001),
        open,
    }
}

pub static DETECTORS: &[Detector] = &[
    on_i(
        "ThermalTake Argent Mouse Bungee",
        VID,
        0x8020,
        1,
        open_argent,
    ),
    on_i(
        "ThermalTake Argent MP1 Mousepad",
        VID,
        0x9011,
        1,
        open_argent,
    ),
    on_i(
        "ThermalTake Argent E1 Headphone Stand",
        VID,
        0x8002,
        1,
        open_argent,
    ),
    on_i(
        "ThermalTake Argent HS1 Headphone Stand",
        VID,
        0x8022,
        1,
        open_argent,
    ),
    on_i("ThermalTake DPSG PSU", VID, 0x2329, 0, open_psu),
    on_i("ThermalTake DPSG PSU", VID, 0x2329, -1, open_psu),
    ipu("ThermalTake LEDBOX Pacific", 0x219A, open_ledbox),
    ipu("ThermalTake LEDBOX Pacific", 0x219B, open_ledbox),
    ipu("ThermalTake LEDBOX Pacific", 0x219C, open_ledbox),
    ipu("ThermalTake LEDBOX Pacific", 0x219D, open_ledbox),
    ipu("ThermalTake LEDBOX Pacific", 0x219E, open_ledbox),
    ipu("ThermalTake LEDBOX Pacific", 0x219F, open_ledbox),
    ipu("ThermalTake Level 20 Desk", 0x07D1, open_desk),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;

    fn model(pid: u16) -> &'static Argent {
        MODELS.iter().find(|m| m.pid == pid).unwrap()
    }

    #[test]
    fn argent_tables_are_consistent() {
        for m in &MODELS {
            assert_eq!(m.entries.len(), m.chunks.iter().sum::<usize>());
            assert!(
                m.entries
                    .iter()
                    .all(|&(_, led)| (led as usize) < m.pos.len())
            );
            assert!(m.chunks.iter().all(|&n| 5 + 4 * n <= LEN));
            assert!(
                m.pos
                    .iter()
                    .all(|&(x, y)| (x as u32) < m.w && (y as u32) < m.h)
            );
        }
    }

    #[test]
    fn bungee_logo_fills_three_slots() {
        let mut c = vec![BLACK; 11];
        c[10] = Color(7, 8, 9);
        let p = argent_packets(model(0x8020), &c);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0][..5], [0x00, 0xC0, 0x01, 13, 0x00]);
        assert_eq!(
            p[0][5 + 4 * 10..5 + 4 * 13],
            [10, 7, 8, 9, 11, 7, 8, 9, 12, 7, 8, 9]
        );
    }

    #[test]
    fn e1_remaps_slots_in_two_chunks() {
        let c: Vec<Color> = (0..25).map(|i| Color(i, 0, 0)).collect();
        let p = argent_packets(model(0x8002), &c);
        assert_eq!(p[0][3], 15);
        assert_eq!(p[0][5..9], [0x00, 0, 0, 0]);
        assert_eq!(p[0][9..13], [0x08, 1, 0, 0]);
        assert_eq!(p[1][3], 10);
        assert_eq!(p[1][5..9], [0x03, 15, 0, 0]);
    }

    #[test]
    fn argent_init_then_frames() {
        let mut io = Rec::default();
        let mut pr = ArgentProto(model(0x8022));
        pr.start(&mut io).unwrap();
        pr.frame(&mut io, &[Color(1, 2, 3); 20]).unwrap();
        assert_eq!(io.ops[0], Op::Write(crate::hid::padded(&[0x41, 0x03], 64)));
        assert_eq!(io.ops[1], Op::Write(crate::hid::padded(&[0x12, 0x22], 64)));
        assert_eq!(io.ops.len(), 4);
        let Op::Write(f) = &io.ops[3] else { panic!() };
        assert_eq!(f[..9], [0x00, 0xC0, 0x01, 5, 0x00, 15, 1, 2, 3]);
    }

    #[test]
    fn psu_sends_grb() {
        let p = psu_packet(&[Color(1, 2, 3); 12]);
        assert_eq!(p.len(), 4 + 36);
        assert_eq!(p[..7], [0x00, 0x30, 0x42, 0x18, 2, 1, 3]);
    }

    #[test]
    fn ledbox_splits_channel_in_two() {
        let c: Vec<Color> = (0..36).map(|i| Color(0, i, 0)).collect();
        let [a, b] = ledbox_packets(2, &c);
        assert_eq!(a[..8], [0x00, 0x32, 0x52, 3, 0x18, 0x03, 1, 0x00]);
        assert_eq!(a.len(), 8 + 57);
        assert_eq!(b[6], 2);
        assert_eq!(b.len(), 8 + 17 * 3);
        assert_eq!(b[8], 19);
        let mut io = Rec::default();
        let mut pr = Ledbox;
        pr.start(&mut io).unwrap();
        assert_eq!(
            io.ops[0],
            Op::Feature(crate::hid::padded(&[0x00, 0xFE, 0x33], 65))
        );
        assert_eq!(io.ops[1], Op::Read(65));
        pr.frame(&mut io, &vec![BLACK; 180]).unwrap();
        assert_eq!(io.ops.len(), 3 + 10);
    }

    #[test]
    fn desk_slots_cover_every_led_once() {
        let pos = desk_positions();
        let mut s = desk_slots();
        assert_eq!(pos.len(), DESK_LEDS);
        assert_eq!(s.len(), DESK_LEDS);
        assert_eq!(pos[100], (99, 1));
        assert_eq!(pos[101], (0, 2));
        assert_eq!(pos[179], (99, 41));
        assert_eq!(pos[180], (1, 42));
        assert_eq!(
            (s[0], s[99], s[100], s[101], s[102], s[179], s[180], s[277]),
            (140, 41, 40, 141, 39, 0, 277, 180)
        );
        s.sort();
        assert!(s.iter().enumerate().all(|(i, &v)| i == v));
    }

    #[test]
    fn desk_packet_places_grb_at_slot() {
        let mut c = vec![BLACK; DESK_LEDS];
        c[0] = Color(1, 2, 3);
        let p = desk_packet(&desk_slots(), &c);
        assert_eq!(p.len(), 5 + 834);
        assert_eq!(p[..5], DESK_HEAD);
        assert_eq!(p[5 + 140 * 3..5 + 141 * 3], [2, 1, 3]);
    }
}
