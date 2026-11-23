//! Roccat and Turtle Beach peripherals.

use super::common::{line, open_with, positioned};
use crate::hid::{Io, Proto, open_sibling, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

const ROCCAT: u16 = 0x1E7D;
const TB: u16 = 0x10F5;
const FAMILY: &str = "roccat";

const fn det(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: Option<i32>,
    usage_page: Option<u16>,
    usage: Option<u16>,
    open: foxden_hid::registry::Open,
) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface,
        usage_page,
        usage,
        open,
    }
}

fn id(info: &Info) -> String {
    info.device_id(FAMILY)
}

fn black(n: usize) -> Vec<Color> {
    vec![Color::BLACK; n]
}

// ---- Burst Pro Air (wired) ----

const BURST_LEN: usize = 30;
const BURST_POS: [(u8, u8); 4] = [(1, 0), (0, 1), (2, 1), (1, 3)];

pub fn burst_init() -> Vec<Vec<u8>> {
    vec![
        vec![0x06, 0x00, 0x35, 0x07],
        vec![
            0x06, 0x00, 0x43, 0x07, 0x0C, 0x11, 0x0D, 0x00, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A,
            0x0A,
        ],
        vec![0x06, 0x00, 0x4E, 0x06, 0x04, 0x01, 0x01, 0x01, 0xFF],
        vec![0x06, 0x00, 0x00, 0x04],
        vec![0x06, 0x00, 0x00, 0x05],
    ]
}

pub fn burst_frame(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x06, 0x00, 0x4D, 0x06, 0x0C];
    p.extend(colors.iter().take(4).flat_map(|c| rgb(*c)));
    p
}

struct BurstAir;

impl Proto for BurstAir {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for _ in 0..2 {
            for p in burst_init() {
                io.feature(&p, BURST_LEN)?;
                io.sleep(70);
            }
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&burst_frame(colors), BURST_LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(4));
    }
}

fn open_burst(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(&id(info), d.name, DeviceKind::Mouse, 3, 4, &BURST_POS);
    open_with(api, info, FAMILY, dev, BurstAir)
}

// ---- Kone XP Air (wireless, wired) and its Turtle Beach twin ----

const XP_POS: [(u8, u8); 5] = [(1, 0), (0, 1), (2, 1), (0, 2), (2, 2)];

/// (pid, target flag, frame tag) per product.
const XP_AIR: [(u16, u8, [u8; 3]); 4] = [
    (0x2CB6, 0x01, [0, 0, 0]),
    (0x5017, 0x01, [0, 0, 0]),
    (0x2CB2, 0x00, [0x6D, 0x70, 0x61]),
    (0x5015, 0x00, [0x6D, 0x70, 0x61]),
];

pub fn xp_air_init(flag: u8) -> Vec<Vec<u8>> {
    vec![
        vec![0x06, flag, 0x13, 0x07, 0x02],
        vec![0x06, 0x00, 0x00, 0x04],
        vec![0x06, 0x00, 0x00, 0x05],
        vec![0x06, flag, 0x35, 0x07],
        vec![0x06, flag, 0x00, 0x04],
        vec![0x06, flag, 0x00, 0x05],
        vec![0x06, flag, 0x4E, 0x06, 0x04, 0x01, 0x01, 0x01, 0xFF],
        vec![0x06, flag, 0x44, 0x07],
        vec![0x06, flag, 0x49, 0x06, 0x01, 0x04],
        vec![0x06, flag, 0x44, 0x07],
    ]
}

pub fn xp_air_frame(flag: u8, tag: [u8; 3], colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x06, flag, 0x4D, 0x06, 0x15, tag[0], tag[1], tag[2]];
    p.extend(colors.iter().take(5).flat_map(|c| rgb(*c)));
    p
}

struct XpAir {
    flag: u8,
    tag: [u8; 3],
}

impl Proto for XpAir {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for p in xp_air_init(self.flag) {
            io.feature(&p, 30)?;
            io.sleep(30);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&xp_air_frame(self.flag, self.tag, colors), 30)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(5));
    }
}

fn open_xp_air(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let &(_, flag, tag) = XP_AIR
        .iter()
        .find(|v| v.0 == info.pid)
        .ok_or("unknown Kone XP Air")?;
    let dev = positioned(&id(info), d.name, DeviceKind::Mouse, 3, 3, &XP_POS);
    open_with(api, info, FAMILY, dev, XpAir { flag, tag })
}

// ---- Kone XP Air dock ----

const DOCK_POS: [(u8, u8); 4] = [(0, 0), (0, 1), (1, 1), (1, 0)];

#[rustfmt::skip]
const DOCK_SETUP: [u8; 39] = [
    0x02, 0x00, 0x09, 0x12, 0xFA, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x09, 0x12, 0xFA, 0x00, 0xFF, 0x7D,
    0x00, 0xFF, 0x09, 0x12, 0xFA, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x09, 0x12, 0xFA, 0x00, 0x7D, 0xFF,
    0x00, 0xFF, 0x00, 0x00, 0x01, 0xFF, 0x00,
];

pub fn dock_frame(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x03];
    for c in colors.iter().take(4) {
        p.extend(rgb(*c));
        p.push(0xFF);
    }
    p
}

struct Dock;

impl Proto for Dock {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&DOCK_SETUP, 39)?;
        io.feature(&[0x01, 0xFF, 0xFF], 5)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&dock_frame(colors), 17)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(4));
    }
}

fn open_dock(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(&id(info), d.name, DeviceKind::Other, 2, 2, &DOCK_POS);
    open_with(api, info, FAMILY, dev, Dock)
}

// ---- Kain 200 (wired, wireless) ----

const KAIN_POS: [(u8, u8); 2] = [(1, 0), (1, 2)];

pub fn kain200_frame(wireless: bool, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x08, 0x09, 0x33, 0x00];
    p.extend(colors.iter().take(2).flat_map(|c| rgb(*c)));
    p.resize(10, 0);
    let ck = if wireless {
        let v = |i: usize| p[i] as i32;
        ((v(4) - v(5) + v(6) - v(7) + v(8) - v(9) - 0x32).abs() & 0xFF) as u8
    } else {
        p.iter().fold(0, |a, b| a ^ b)
    };
    p.push(ck);
    p
}

struct Kain200 {
    wireless: bool,
}

impl Proto for Kain200 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&kain200_frame(self.wireless, colors), 22)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(2));
    }
}

fn open_kain200(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(&id(info), d.name, DeviceKind::Mouse, 3, 3, &KAIN_POS);
    open_with(
        api,
        info,
        FAMILY,
        dev,
        Kain200 {
            wireless: info.pid == 0x2D60,
        },
    )
}

// ---- Kova Aimo ----

const KOVA_POS: [(u8, u8); 2] = [(1, 0), (1, 3)];
const KOVA_SLOTS: [usize; 2] = [1, 0];
const SWITCH_ON: [u8; 6] = [0x0E, 0x06, 0x01, 0x01, 0x00, 0xFF];
const SWITCH_OFF: [u8; 6] = [0x0E, 0x06, 0x00, 0x00, 0x00, 0xFF];

pub fn kova_frame(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0u8; 8];
    p[..2].copy_from_slice(&[0x0A, 0x08]);
    for (c, &s) in colors.iter().zip(&KOVA_SLOTS) {
        p[2 + 3 * s..5 + 3 * s].copy_from_slice(&rgb(*c));
    }
    p
}

struct KovaAimo;

impl Proto for KovaAimo {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&SWITCH_ON, 6)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&kova_frame(colors), 8)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(2));
        let _ = io.feature(&SWITCH_OFF, 6);
    }
}

fn open_kova(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(&id(info), d.name, DeviceKind::Mouse, 3, 4, &KOVA_POS);
    open_with(api, info, FAMILY, dev, KovaAimo)
}

// ---- Elo 7.1 Air ----

pub fn elo_air_init() -> Vec<Vec<u8>> {
    vec![
        vec![0x00, 0xFF, 0x02],
        vec![0x00, 0xFF, 0x03, 0x00, 0x01, 0x00, 0x01],
        vec![0x00, 0xFF, 0x04],
        vec![0x00, 0xFF, 0x01],
    ]
}

pub fn elo_air_frame(c: Color) -> Vec<u8> {
    let mut p = vec![0x00, 0xFF, 0x04, 0x00, 0x00];
    p.extend(rgb(c));
    p
}

/// The headset announces it came back with E0 06 03 .. 01.
fn elo_reconnected(r: &[u8]) -> bool {
    r.len() > 5 && r[1] == 0xE0 && r[2] == 0x06 && r[3] == 0x03 && r[5] == 0x01
}

/// Wireless headsets: reinit some time after the dongle reports the headset back.
#[derive(Default)]
struct Rejoin {
    seen: Option<Instant>,
}

impl Rejoin {
    fn poll(&mut self, io: &mut dyn Io, back: fn(&[u8]) -> bool) -> Result<(), String> {
        for _ in 0..64 {
            let r = io.read(65, 0)?;
            if r.is_empty() {
                break;
            }
            if back(&r) {
                self.seen = Some(Instant::now());
            }
        }
        Ok(())
    }
    fn due(&mut self, wait: Duration) -> bool {
        if self.seen.is_some_and(|t| t.elapsed() >= wait) {
            self.seen = None;
            return true;
        }
        false
    }
}

#[derive(Default)]
struct EloAir {
    rejoin: Rejoin,
}

impl EloAir {
    fn init(io: &mut dyn Io) -> Result<(), String> {
        for p in elo_air_init() {
            io.write(&p, 64)?;
            io.sleep(10);
        }
        let _ = io.read(65, 10);
        Ok(())
    }
}

impl Proto for EloAir {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        Self::init(io)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&elo_air_frame(colors[0]), 65)?;
        self.rejoin.poll(io, elo_reconnected)?;
        if self.rejoin.due(Duration::from_secs(20)) {
            Self::init(io)?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(2000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&elo_air_frame(Color::BLACK), 65);
    }
}

fn open_elo_air(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(&id(info), d.name, DeviceKind::Other, 1);
    open_with(api, info, FAMILY, dev, EloAir::default())
}

// ---- Syn Pro Air ----

const SYN_KEEPALIVE: Duration = Duration::from_secs(8);

pub fn syn_init() -> Vec<Vec<u8>> {
    let mut first = vec![0x00, 0xFE, 0x01, 0x00, 0x00, 0x00, 0x09, 0x08, 0x02, 0x00];
    first.extend([0xFF; 9]);
    vec![first, vec![0x00, 0xFE, 0x02, 0x00, 0x00, 0x01]]
}

pub fn syn_frame(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x00, 0xFE, 0x03, 0x00, 0x00];
    p.extend(colors.iter().take(2).flat_map(|c| rgb(*c)));
    p
}

fn syn_reconnected(r: &[u8]) -> bool {
    r.len() > 5 && r[1] == 0xE6 && r[2] == 0x06 && r[3] == 0x04 && r[5] == 0x01
}

struct SynAir {
    rejoin: Rejoin,
    ping: Instant,
}

impl SynAir {
    fn init(io: &mut dyn Io) -> Result<(), String> {
        syn_init().iter().try_for_each(|p| io.write(p, 64))
    }
}

impl Proto for SynAir {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        Self::init(io)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.rejoin.poll(io, syn_reconnected)?;
        if self.rejoin.due(Duration::from_secs(1)) {
            Self::init(io)?;
        }
        if self.ping.elapsed() >= SYN_KEEPALIVE {
            io.write(&[0x00, 0xFE, 0x04], 64)?;
            self.ping = Instant::now();
        }
        io.write(&syn_frame(colors), 65)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(SYN_KEEPALIVE.as_millis() as u64)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&syn_frame(&black(2)), 65);
    }
}

fn open_syn(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &id(info),
        d.name,
        DeviceKind::Other,
        7,
        7,
        &[(1, 0), (2, 0)],
    );
    let proto = SynAir {
        rejoin: Rejoin::default(),
        ping: Instant::now(),
    };
    open_with(api, info, FAMILY, dev, proto)
}

// ---- Torch microphone ----

/// The Torch packs its colour into three bytes after the 0x80 command.
pub fn torch_frame(c: Color) -> Vec<u8> {
    vec![
        0x00,
        0x80,
        c.0 >> 3,
        (c.0 >> 6) | (c.1 >> 2),
        (c.1 >> 7) | (c.2 >> 2),
    ]
}

struct Torch;

impl Proto for Torch {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for p in [
            &[0x00, 0x80, 0x80][..],
            &[0x00, 0x80, 0x02],
            &[0x00, 0x80, 0x02],
            &[0x00],
        ] {
            io.write(p, 6)?;
            io.sleep(100);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&torch_frame(colors[0]), 6)?;
        io.sleep(7);
        io.write(&[0x00], 6)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&torch_frame(Color::BLACK), 6);
    }
}

fn open_torch(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(&id(info), d.name, DeviceKind::Other, 1);
    open_with(api, info, FAMILY, dev, Torch)
}

// ---- Vulcan II Mini Air (Roccat and Turtle Beach) ----

const MINI_W: u32 = 15;
const MINI_H: u32 = 5;

#[rustfmt::skip]
const MINI_POS: [(u8, u8); 98] = [
    (0,0),(1,0),(2,0),(3,0),(4,0),(5,0),(6,0),(7,0),(8,0),(9,0),(10,0),(11,0),(12,0),(13,0),(14,0),
    (0,1),(1,1),(2,1),(3,1),(4,1),(5,1),(6,1),(7,1),(8,1),(9,1),(10,1),(11,1),(12,1),(13,1),(14,1),
    (0,2),(1,2),(2,2),(3,2),(4,2),(5,2),(6,2),(7,2),(8,2),(9,2),(10,2),(11,2),(12,2),(13,2),(14,2),
    (0,3),(1,3),(2,3),(3,3),(4,3),(5,3),(6,3),(7,3),(8,3),(9,3),(10,3),(11,3),(12,3),(13,3),(14,3),
    (0,4),(1,4),(2,4),(6,4),(9,4),(10,4),(11,4),(12,4),(13,4),(14,4),
    (0,0),(1,0),(2,0),(3,0),(4,0),(5,0),(6,0),(7,0),(8,0),(9,0),(10,0),(11,0),(12,0),
    (0,1),(5,1),(6,1),(7,1),(8,1),
    (0,2),
    (2,3),(3,3),(4,3),(5,3),(6,3),(7,3),(8,3),(9,3),(10,3),
];

#[rustfmt::skip]
const MINI_SLOTS: [u8; 98] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44,
    45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 66, 69, 70, 71, 72, 73, 74,
    75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87,
    88, 89, 90, 91, 92,
    93,
    100, 101, 102, 94, 95, 96, 97, 98, 99,
];

const MINI_CHUNK: usize = 45;
/// (zone, packets) the plugin streams.
const MINI_ZONES: [(u8, u8); 2] = [(1, 5), (2, 2)];

pub fn mini_air_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let total: usize = MINI_ZONES.iter().map(|z| z.1 as usize).sum();
    let mut data = vec![0u8; total * MINI_CHUNK];
    for (c, &s) in colors.iter().zip(&MINI_SLOTS) {
        let o = s as usize * 3;
        data[o..o + 3].copy_from_slice(&rgb(*c));
    }
    let mut chunks = data.chunks(MINI_CHUNK);
    let mut out = Vec::new();
    for (zone, count) in MINI_ZONES {
        for n in 0..count {
            let mut p = vec![0x00, 0x07, 0x31, 0x33, zone, n];
            p.extend(chunks.next().unwrap_or_default());
            out.push(p);
        }
    }
    out
}

struct MiniAir;

impl Proto for MiniAir {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in mini_air_packets(colors) {
            io.write(&p, 65)?;
            io.sleep(5);
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(MINI_POS.len()));
    }
}

fn open_mini_air(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &id(info),
        d.name,
        DeviceKind::Keyboard,
        MINI_W,
        MINI_H,
        &MINI_POS,
    );
    open_with(api, info, FAMILY, dev, MiniAir)
}

// ---- Turtle Beach AIMO pad wide ----

pub fn aimo_frame(colors: &[Color]) -> Vec<u8> {
    dock_frame(colors).into_iter().take(9).collect()
}

/// Frames go to interface 3; the take-over report goes to the matched collection.
struct AimoPad {
    setup: HidDevice,
}

impl Proto for AimoPad {
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Io::feature(&mut self.setup, &[0x01, 0xFF, 0x00, 0x00, 0x00], 5)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&aimo_frame(colors), 9)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(&[0x01, 0x00, 0x00, 0x00, 0x00], 5);
    }
}

fn open_aimo(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let lights = open_sibling(api, info, Some(3), Some(0x0001), Some(0x0000))?;
    let dev = positioned(
        &id(info),
        d.name,
        DeviceKind::Other,
        10,
        3,
        &[(0, 0), (9, 2)],
    );
    let proto = AimoPad {
        setup: info.open(api)?,
    };
    Ok(crate::hid::Driver::new(Box::new(lights), dev, FAMILY, proto).boxed())
}

// ---- Turtle Beach Command Series KB7 ----

#[rustfmt::skip]
const KB7_POS: [(u8, u8); 101] = [
    (2,0),(3,0),(7,0),(8,0),(9,0),(10,0),
    (0,1),(2,1),(3,1),(4,1),(5,1),(6,1),(7,1),(8,1),(9,1),(10,1),(11,1),(12,1),(13,1),(14,1),
    (0,2),(1,2),(2,2),(3,2),(4,2),(5,2),(6,2),(7,2),(8,2),(9,2),(10,2),(11,2),(12,2),(13,2),
    (0,3),(1,3),(2,3),(3,3),(4,3),(5,3),(6,3),(7,3),(8,3),(9,3),(10,3),(11,3),(12,3),(13,3),
    (0,4),(1,4),(2,4),(3,4),(4,4),(5,4),(6,4),(7,4),(8,4),(9,4),(10,4),(11,4),(12,4),(13,4),
    (0,5),(1,5),(2,5),(3,5),(4,5),(5,5),(6,5),(7,5),(8,5),(9,5),(10,5),(11,5),(12,5),(13,5),(14,5),
    (0,6),(1,6),(2,6),(3,6),(5,6),(6,6),(7,6),(8,6),(9,6),(10,6),(11,6),(12,6),(13,6),(14,6),
    (0,7),(1,7),(3,7),(5,7),(7,7),(8,7),(9,7),(11,7),(13,7),(14,7),
];

#[rustfmt::skip]
const KB7_KEYS: [u8; 101] = [
    85, 86, 87, 88, 89, 90,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,
    14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
    42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55,
    56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70,
    71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84,
    96, 97, 98, 99, 100, 101, 102, 103, 104, 105,
];

const KB7_PACKETS: usize = 6;
const KB7_CHUNK: usize = 60;

/// Keys sit in blocks of 16: a block's 16 reds, then greens, then blues.
pub fn kb7_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let mut data = vec![0u8; KB7_PACKETS * KB7_CHUNK];
    for (c, &k) in colors.iter().zip(&KB7_KEYS) {
        let i = (k as usize / 16) * 48 + k as usize % 16;
        data[i] = c.0;
        data[i + 16] = c.1;
        data[i + 32] = c.2;
    }
    data.chunks(KB7_CHUNK)
        .enumerate()
        .map(|(n, chunk)| {
            let len = if n == 0 { [0x68, 0x01] } else { [0, 0] };
            let mut p = vec![0x00, 0xA1, n as u8 + 1, len[0], len[1]];
            p.extend(chunk);
            p
        })
        .collect()
}

struct Kb7 {
    config: HidDevice,
}

impl Kb7 {
    fn software(&mut self, on: bool) -> Result<(), String> {
        Io::feature(&mut self.config, &[0x0E, 0x05, on as u8, 0x00, 0x00], 5)
    }
}

impl Proto for Kb7 {
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        self.software(true)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        kb7_packets(colors).iter().try_for_each(|p| io.write(p, 65))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &black(KB7_KEYS.len()));
        let _ = self.software(false);
    }
}

fn open_kb7(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let config = open_sibling(api, info, Some(2), None, None)?;
    let dev = positioned(&id(info), d.name, DeviceKind::Keyboard, 15, 8, &KB7_POS);
    open_with(api, info, FAMILY, dev, Kb7 { config })
}

pub static DETECTORS: &[Detector] = &[
    det(
        "Roccat Burst Pro Air (Wired)",
        ROCCAT,
        0x2CAB,
        Some(2),
        None,
        Some(0xFF00),
        open_burst,
    ),
    det(
        "Roccat Kone XP Air",
        ROCCAT,
        0x2CB6,
        Some(2),
        None,
        Some(0xFF00),
        open_xp_air,
    ),
    det(
        "Roccat Kone XP Air (Wired)",
        ROCCAT,
        0x2CB2,
        Some(2),
        None,
        Some(0xFF00),
        open_xp_air,
    ),
    det(
        "Turtle Beach Kone XP Air",
        TB,
        0x5017,
        Some(2),
        None,
        Some(0xFF00),
        open_xp_air,
    ),
    det(
        "Turtle Beach Kone XP Air (Wired)",
        TB,
        0x5015,
        Some(2),
        None,
        Some(0xFF00),
        open_xp_air,
    ),
    det(
        "Roccat Kone XP Air Dock",
        ROCCAT,
        0x2CB8,
        Some(0),
        None,
        Some(0x0001),
        open_dock,
    ),
    det(
        "Turtle Beach Kone XP Air Dock",
        TB,
        0x5019,
        Some(0),
        None,
        Some(0x0001),
        open_dock,
    ),
    det(
        "Roccat Kain 200",
        ROCCAT,
        0x2D5F,
        Some(2),
        Some(0xFF01),
        Some(0x0001),
        open_kain200,
    ),
    det(
        "Roccat Kain 200 (Wireless)",
        ROCCAT,
        0x2D60,
        Some(2),
        Some(0xFF01),
        Some(0x0001),
        open_kain200,
    ),
    det(
        "Roccat Kova Aimo",
        ROCCAT,
        0x2CF1,
        Some(0),
        None,
        Some(0x0000),
        open_kova,
    ),
    det(
        "Roccat Elo 7.1 Air",
        ROCCAT,
        0x3A37,
        Some(0),
        None,
        None,
        open_elo_air,
    ),
    det(
        "Roccat Syn Pro Air",
        ROCCAT,
        0x3A3B,
        Some(0),
        None,
        None,
        open_syn,
    ),
    det(
        "Roccat Torch",
        ROCCAT,
        0x3A56,
        Some(3),
        None,
        None,
        open_torch,
    ),
    det(
        "Roccat Vulcan II Mini Air",
        ROCCAT,
        0x6978,
        Some(1),
        Some(0x00FF),
        Some(0x0001),
        open_mini_air,
    ),
    det(
        "Roccat Vulcan II Mini Air (Wireless)",
        ROCCAT,
        0x3AA9,
        Some(1),
        Some(0x00FF),
        Some(0x0001),
        open_mini_air,
    ),
    det(
        "Turtle Beach Vulcan II Mini Air",
        TB,
        0x501E,
        Some(1),
        Some(0x00FF),
        Some(0x0001),
        open_mini_air,
    ),
    det(
        "Turtle Beach Vulcan II Mini Air (Wireless)",
        TB,
        0x5021,
        Some(1),
        Some(0x00FF),
        Some(0x0001),
        open_mini_air,
    ),
    det(
        "Turtle Beach AIMO Pad Wide",
        TB,
        0x5026,
        Some(0),
        Some(0xFF01),
        None,
        open_aimo,
    ),
    det(
        "Turtle Beach Command Series KB7",
        TB,
        0x5038,
        Some(1),
        None,
        None,
        open_kb7,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;
    use crate::hid::padded;

    const A: Color = Color(1, 2, 3);
    const B: Color = Color(4, 5, 6);

    fn feat(d: &[u8], n: usize) -> Op {
        Op::Feature(padded(d, n))
    }

    #[test]
    fn burst_air_init_twice_then_frame() {
        let mut r = Rec::default();
        BurstAir.start(&mut r).unwrap();
        assert_eq!(r.ops.len(), 10);
        assert_eq!(r.ops[0], feat(&[0x06, 0x00, 0x35, 0x07], 30));
        BurstAir.frame(&mut r, &[A, B, A, B]).unwrap();
        assert_eq!(
            r.ops[10],
            feat(
                &[
                    0x06, 0, 0x4D, 0x06, 0x0C, 1, 2, 3, 4, 5, 6, 1, 2, 3, 4, 5, 6
                ],
                30
            )
        );
    }

    #[test]
    fn xp_air_flags_and_tags() {
        assert_eq!(xp_air_init(1)[1], vec![0x06, 0x00, 0x00, 0x04]);
        assert_eq!(xp_air_init(1)[3][1], 1);
        let f = xp_air_frame(0, [0x6D, 0x70, 0x61], &[A; 5]);
        assert_eq!(
            f[..11],
            [0x06, 0, 0x4D, 0x06, 0x15, 0x6D, 0x70, 0x61, 1, 2, 3]
        );
        assert_eq!(f.len(), 23);
        assert!(
            XP_AIR
                .iter()
                .all(|v| DETECTORS.iter().any(|d| d.pid == v.0))
        );
    }

    #[test]
    fn dock_frame_layout() {
        let f = dock_frame(&[A, B, A, B]);
        assert_eq!(f[..9], [0x03, 1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
        assert_eq!(f.len(), 17);
    }

    #[test]
    fn kain200_checksums() {
        let w = kain200_frame(false, &[A, B]);
        assert_eq!(w[..10], [0x08, 0x09, 0x33, 0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(w[10], 0x08 ^ 0x09 ^ 0x33 ^ 1 ^ 2 ^ 3 ^ 4 ^ 5 ^ 6);
        let x = kain200_frame(true, &[A, B]);
        assert_eq!(
            x[10],
            (1 - 2 + 3 - 4 + 5 - 6 - 0x32i32).unsigned_abs() as u8
        );
    }

    #[test]
    fn kova_swaps_slots() {
        assert_eq!(kova_frame(&[A, B]), vec![0x0A, 0x08, 4, 5, 6, 1, 2, 3]);
        let mut r = Rec::default();
        KovaAimo.stop(&mut r);
        assert_eq!(r.ops[1], feat(&SWITCH_OFF, 6));
    }

    #[test]
    fn headset_frames() {
        assert_eq!(elo_air_frame(A), vec![0, 0xFF, 0x04, 0, 0, 1, 2, 3]);
        assert_eq!(
            syn_frame(&[A, B])[..11],
            [0, 0xFE, 0x03, 0, 0, 1, 2, 3, 4, 5, 6]
        );
        assert!(elo_reconnected(&[0, 0xE0, 0x06, 0x03, 0, 0x01]));
        assert!(syn_reconnected(&[0, 0xE6, 0x06, 0x04, 0, 0x01]));
        assert_eq!(syn_init()[0].len(), 19);
    }

    #[test]
    fn elo_reinits_after_reconnect_wait() {
        let mut e = EloAir::default();
        let mut r = Rec::default();
        r.replies.push_back(vec![0, 0xE0, 0x06, 0x03, 0, 0x01]);
        e.frame(&mut r, &[A]).unwrap();
        assert!(e.rejoin.seen.is_some());
        e.rejoin.seen = Some(Instant::now() - Duration::from_secs(21));
        r.ops.clear();
        e.frame(&mut r, &[A]).unwrap();
        assert!(r.ops.contains(&Op::Write(padded(&[0, 0xFF, 0x02], 64))));
    }

    #[test]
    fn torch_packing() {
        assert_eq!(
            torch_frame(Color(0xFF, 0xFF, 0xFF)),
            vec![0, 0x80, 0x1F, 0x3F, 0x3F]
        );
        let mut r = Rec::default();
        Torch.frame(&mut r, &[A]).unwrap();
        assert_eq!(r.ops[1], Op::Write(padded(&[0x00], 6)));
    }

    #[test]
    fn mini_air_slots_and_zones() {
        let mut c = black(MINI_POS.len());
        c[70] = A;
        c[89] = B;
        let p = mini_air_packets(&c);
        assert_eq!(p.len(), 7);
        assert_eq!(p[4][..6], [0, 0x07, 0x31, 0x33, 1, 4]);
        assert_eq!(p[5][..6], [0, 0x07, 0x31, 0x33, 2, 0]);
        // slot 75 starts zone 2
        assert_eq!(p[5][6..9], [1, 2, 3]);
        // LED 89 drives slot 100, byte 300 of the stream
        assert_eq!(p[6][6 + 300 - 270..9 + 300 - 270], [4, 5, 6]);
        assert!(p.iter().all(|x| x.len() == 6 + MINI_CHUNK));
    }

    #[test]
    fn kb7_planar_blocks() {
        let mut c = black(KB7_KEYS.len());
        c[0] = A; // key 85: block 5, column 5
        c[6] = B; // key 0
        let p = kb7_packets(&c);
        assert_eq!(p.len(), 6);
        assert_eq!(p[0][..5], [0, 0xA1, 1, 0x68, 0x01]);
        assert_eq!(p[1][..5], [0, 0xA1, 2, 0, 0]);
        assert_eq!([p[0][5], p[0][5 + 16], p[0][5 + 32]], [4, 5, 6]);
        let i = 5 * 48 + 5;
        let at = |o: usize| p[o / 60][5 + o % 60];
        assert_eq!([at(i), at(i + 16), at(i + 32)], [1, 2, 3]);
    }

    #[test]
    fn layouts_match_tables() {
        assert_eq!(MINI_POS.len(), MINI_SLOTS.len());
        assert_eq!(KB7_POS.len(), KB7_KEYS.len());
        assert!(DETECTORS.iter().all(|d| d.vid == ROCCAT || d.vid == TB));
    }
}
