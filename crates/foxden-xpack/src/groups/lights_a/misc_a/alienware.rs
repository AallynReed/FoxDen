//! Alienware laptop, mouse and monitors.

use super::super::common::{KEYBOARD_RECT, MOUSE_RECT, SMALL_RECT, full, keyed};
use crate::hid::{Driver, Io, Proto, on_i, open_io};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::{HidApi, HidDevice};

/// Laptop zone bitmasks, one per LED in order.
const R3_ZONES: [u32; 13] = [
    0x000001, 0x000002, 0x000004, 0x000008, 0x000020, 0x000040, 0x000080, 0x000100, 0x000400,
    0x000800, 0x001000, 0x002000, 0x004000,
];
const R3_CELLS: [[u8; 2]; 13] = [
    [5, 2],
    [4, 2],
    [3, 2],
    [2, 2],
    [3, 0],
    [3, 1],
    [3, 1],
    [3, 3],
    [0, 3],
    [6, 3],
    [0, 0],
    [6, 0],
    [1, 2],
];
const R3_LEN: usize = 12;

/// 15 R3: begin `[2, 7, 4]`, per zone `[2, 3, n, mask(3), r, g, b]` and `[2, 4]`, then apply `[2, 5]`.
pub fn r3_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let mut v = vec![vec![0x02, 0x07, 0x04]];
    for (i, (m, c)) in R3_ZONES.iter().zip(colors).enumerate() {
        let [_, a, b, d] = m.to_be_bytes();
        v.push(vec![0x02, 0x03, i as u8 + 1, a, b, d, c.0, c.1, c.2]);
        v.push(vec![0x02, 0x04]);
    }
    v.push(vec![0x02, 0x05]);
    v
}

struct R3;

impl Proto for R3 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (i, p) in r3_packets(colors).iter().enumerate() {
            io.write(p, R3_LEN)?;
            if i == 0 {
                io.sleep(1);
            }
        }
        Ok(())
    }
}

fn open_r3(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("alienware-15r3"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [7, 5],
        &R3_CELLS,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "alienware-15r3", R3).boxed())
}

/// AW558 LED slots per position.
const AW558_SLOTS: [usize; 3] = [2, 1, 0];
const AW558_CELLS: [[u8; 2]; 3] = [[1, 2], [2, 0], [0, 0]];

/// `[0, 0x0D, 0x5A, 3, 0, rgb x3 by slot, ..., 0xA5 at 14]`, 64 bytes.
pub fn aw558_packet(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0u8; 15];
    p[1..5].copy_from_slice(&[0x0D, 0x5A, 0x03, 0x00]);
    for (s, c) in AW558_SLOTS.iter().zip(colors) {
        p[5 + s * 3..8 + s * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    p[14] = 0xA5;
    p
}

struct Aw558;

impl Proto for Aw558 {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        let mut soft = [0u8; 14];
        soft[..4].copy_from_slice(&[0x0D, 0x5A, 0x21, 0x02]);
        soft[13] = 0xA5;
        let _ = io.write(&soft, 64);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&aw558_packet(colors), 64)?;
        io.sleep(1);
        Ok(())
    }
}

fn open_aw558(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("alienware-aw558"),
        d.name,
        DeviceKind::Mouse,
        MOUSE_RECT,
        [3, 4],
        &AW558_CELLS,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "alienware-aw558", Aw558).boxed())
}

/// Monitor models: pid, generation, size, LED ids, positions.
type Monitor = (u16, u8, [u32; 2], &'static [u8], &'static [[u8; 2]]);

const MONITORS: &[Monitor] = &[
    (
        0x274A,
        1,
        [17, 7],
        &[1, 2, 4, 8],
        &[[0, 0], [8, 3], [8, 6], [16, 6]],
    ),
    (
        0x274B,
        1,
        [17, 7],
        &[1, 2, 4, 8],
        &[[0, 0], [8, 3], [8, 6], [16, 6]],
    ),
    (0x274C, 1, [1, 1], &[1], &[[0, 0]]),
    (
        0x1008,
        2,
        [17, 5],
        &[4, 2, 1, 8],
        &[[8, 4], [8, 0], [0, 0], [16, 4]],
    ),
    (
        0x100B,
        2,
        [17, 5],
        &[1, 2, 4, 8],
        &[[8, 4], [8, 0], [0, 0], [16, 4]],
    ),
    (0x1013, 2, [8, 7], &[1, 2, 8], &[[1, 2], [4, 2], [7, 6]]),
    (0x1014, 2, [8, 7], &[1, 2, 8], &[[1, 2], [4, 2], [7, 6]]),
];

/// Gen 1: `[0xFF, 0x92, 0x48, 5, 0, 4, led, r, g, b]` padded with 0xFF.
pub fn gen1_packet(led: u8, c: Color) -> Vec<u8> {
    let mut p = vec![0xFF; 65];
    p[1..10].copy_from_slice(&[0x92, 0x48, 0x05, 0x00, 0x04, led, c.0, c.1, c.2]);
    p
}

/// Gen 2: `[0xFF, 0x92, 0x37, 0x0A, 0, 0x51, 0x87, 0xD0, 4, led, r, g, b, level, xor]` padded with 0xFF.
pub fn gen2_packet(led: u8, c: Color, level: u8) -> Vec<u8> {
    let mut p = vec![0xFF; 65];
    p[1..14].copy_from_slice(&[
        0x92, 0x37, 0x0A, 0x00, 0x51, 0x87, 0xD0, 0x04, led, c.0, c.1, c.2, level,
    ]);
    p[14] = p[5..=13].iter().fold(110u8, |a, b| a ^ b);
    p
}

struct Monitors {
    generation: u8,
    leds: &'static [u8],
    level: u8,
}

impl Proto for Monitors {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.generation == 1 {
            let mut p = vec![0xFF; 65];
            p[1..5].copy_from_slice(&[0x95, 0, 0, 0]);
            io.write(&p, 65)
        } else {
            io.write(
                &[0x00, 0x92, 0x37, 0x05, 0x00, 0x51, 0x82, 0xD0, 0xF4, 0x99],
                65,
            )
        }
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (led, c) in self.leds.iter().zip(colors) {
            let p = if self.generation == 1 {
                gen1_packet(*led, *c)
            } else {
                gen2_packet(*led, *c, self.level)
            };
            io.write(&p, 65)?;
        }
        Ok(())
    }
}

fn open_monitor(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let m = MONITORS
        .iter()
        .find(|m| m.0 == info.pid)
        .ok_or("unknown Alienware monitor")?;
    let dev = keyed(
        &info.device_id("alienware-monitor"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        m.2,
        m.4,
    );
    let level = if info.pid == 0x1013 { 0x64 } else { 0x23 };
    let p = Monitors {
        generation: m.1,
        leds: m.3,
        level,
    };
    Ok(Driver::new(open_io(api, info)?, dev, "alienware-monitor", p).boxed())
}

/// Gen 3 authenticated monitors: OEM key and checksum style.
struct Gen3Model {
    key: &'static [u8],
    uniform: bool,
}

const AW3423DWF: Gen3Model = Gen3Model {
    key: &[
        0xF5, 0x3F, 0xC1, 0x39, 0x44, 0x3A, 0x31, 0x79, 0x0D, 0xB1, 0x82, 0x76,
    ],
    uniform: false,
};
const AW2724DM: Gen3Model = Gen3Model {
    key: &[
        0x7E, 0x7F, 0x18, 0x00, 0x2B, 0x33, 0xFB, 0xFD, 0xE3, 0x64, 0xD0, 0xC0, 0x04, 0xB0, 0xDA,
        0xC7, 0xB5,
    ],
    uniform: true,
};
const GEN3_LEN: usize = 192;
/// Whole-monitor LED id (the plugin's default, non-individual mode).
const GEN3_ALL: u8 = 0x0B;
/// The plugin's default pause after each colour write.
const GEN3_DELAY_MS: u64 = 200;

/// Session key: an 8-byte key unfolded from the OEM key, mixed with the 16-byte token.
pub fn gen3_key(oem: &[u8], token: &[u8]) -> [u8; 8] {
    let at = |i: usize| oem.get(i).copied().unwrap_or(0);
    let mut key = [0u8; 8];
    let (mut ki, mut oi) = (0, 0);
    while oi < oem.len() && ki < 8 {
        let n = (oem[oi] & 1) + ((oem[oi] & 0x10) >> 4);
        for _ in 0..n {
            if ki >= 8 {
                break;
            }
            key[ki] = at(oi + 1) ^ at(oi);
            oi += 1;
            ki += 1;
        }
        oi += 1;
    }
    if token.len() < 16 {
        return key;
    }
    let odd = (u16::from_be_bytes([token[0], token[15]]).count_ones() & 1) == 1;
    let mut mix = [0u8; 8];
    if odd {
        mix.copy_from_slice(&token[..8]);
        let i = (token[14] & 7) as usize;
        mix[i] ^= token[i + 8];
    } else {
        mix.copy_from_slice(&token[8..16]);
        let i = (token[6] & 7) as usize;
        mix[i] ^= token[i];
    }
    for (k, m) in key.iter_mut().zip(mix) {
        *k ^= m;
    }
    key
}

pub fn gen3_login(key: &[u8; 8]) -> Vec<u8> {
    let mut p = vec![0u8; 73];
    p[1..4].copy_from_slice(&[0x40, 0xE1, 0x02]);
    p[65..73].copy_from_slice(key);
    p
}

pub fn gen3_colour(led: u8, c: Color, uniform: bool) -> Vec<u8> {
    let mut p = vec![0u8; 75];
    p[1] = 0x40;
    p[2] = 0xC6;
    p[7] = 0x0A;
    p[9] = 0x6E;
    p[11] = 0x82;
    p[65..74].copy_from_slice(&[0x51, 0x87, 0xD0, 0x04, led, c.0, c.1, c.2, 0x64]);
    let rgb = c.0 ^ c.1 ^ c.2 ^ 0x64 ^ led;
    p[74] = match (uniform, led) {
        (true, _) => 0x6E ^ 0x51 ^ 0x87 ^ 0xD0 ^ 0x04 ^ rgb,
        (false, GEN3_ALL) => 0x6E ^ rgb,
        _ => 0x51 ^ 0x87 ^ 0xD0 ^ 0x04 ^ rgb,
    };
    p
}

/// Gen 3 talks HID, but its token comes from a GET_REPORT for input report 0.
struct Gen3Io(HidDevice);

impl Io for Gen3Io {
    fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        Io::write(&mut self.0, data, len)
    }
    fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        self.0.feature(data, len)
    }
    fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String> {
        self.0.get_feature(first, len)
    }
    /// Input report 0 fetched with GET_REPORT; the leading report id is dropped.
    fn read(&mut self, len: usize, _timeout_ms: i32) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len + 1];
        let n = self.0.get_input_report(&mut b).map_err(|e| e.to_string())?;
        b.truncate(n);
        if !b.is_empty() {
            b.remove(0);
        }
        Ok(b)
    }
}

struct Gen3(&'static Gen3Model);

impl Proto for Gen3 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&[0x00, 0x40, 0xE1, 0x01], GEN3_LEN)?;
        let token = io.read(GEN3_LEN, 1000)?;
        let key = gen3_key(self.0.key, &token[..token.len().min(16)]);
        io.write(&gen3_login(&key), GEN3_LEN)?;
        io.write(&gen3_colour(GEN3_ALL, colors[0], self.0.uniform), GEN3_LEN)?;
        io.sleep(GEN3_DELAY_MS);
        Ok(())
    }
}

fn open_gen3(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let m = if info.pid == 0x1010 {
        &AW2724DM
    } else {
        &AW3423DWF
    };
    let dev = keyed(
        &info.device_id("alienware-monitor"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [1, 1],
        &[[0, 0]],
    );
    let io = Box::new(Gen3Io(info.open(api)?));
    Ok(Driver::new(io, dev, "alienware-monitor", Gen3(m)).boxed())
}

pub static DETECTORS: &[Detector] = &[
    on_i("Alienware 15 R3", 0x187C, 0x0530, -1, open_r3),
    on_i("Alienware 15 R3", 0x187C, 0x0530, 0, open_r3),
    full(
        "Alienware AW558",
        0x04CA,
        0x00A5,
        1,
        0x000C,
        0x0001,
        open_aw558,
    ),
    full(
        "Alienware AW3418DW",
        0x0424,
        0x274A,
        0,
        0xFF00,
        1,
        open_monitor,
    ),
    full(
        "Alienware AW3418HW",
        0x0424,
        0x274B,
        0,
        0xFF00,
        1,
        open_monitor,
    ),
    full(
        "Alienware AW2518H",
        0x0424,
        0x274C,
        0,
        0xFF00,
        1,
        open_monitor,
    ),
    full(
        "Alienware AW3821DW",
        0x187C,
        0x1008,
        0,
        0xFF00,
        1,
        open_monitor,
    ),
    full(
        "Alienware AW3423DW",
        0x187C,
        0x100B,
        0,
        0xFF00,
        1,
        open_monitor,
    ),
    full(
        "Alienware AW2725DF",
        0x187C,
        0x1014,
        0,
        0xFF00,
        1,
        open_monitor,
    ),
    full(
        "Alienware AW2724DM",
        0x187C,
        0x1010,
        0,
        0xFFDA,
        0x00DA,
        open_gen3,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laptop_zone_packets() {
        let p = r3_packets(&[Color(1, 2, 3); 13]);
        assert_eq!(p.len(), 1 + 13 * 2 + 1);
        assert_eq!(p[0], vec![2, 7, 4]);
        assert_eq!(p[1], vec![2, 3, 1, 0, 0, 1, 1, 2, 3]);
        assert_eq!(p[25], vec![2, 3, 13, 0, 0x40, 0, 1, 2, 3]);
        assert_eq!(p[27], vec![2, 5]);
    }

    #[test]
    fn mouse_and_monitor_packets() {
        let p = aw558_packet(&[Color(1, 1, 1), Color(2, 2, 2), Color(3, 3, 3)]);
        assert_eq!(
            p,
            vec![0, 0x0D, 0x5A, 3, 0, 3, 3, 3, 2, 2, 2, 1, 1, 1, 0xA5]
        );
        let g2 = gen2_packet(1, Color(0, 0, 0), 0x23);
        assert_eq!(g2[14], 110 ^ 0x51 ^ 0x87 ^ 0xD0 ^ 0x04 ^ 1 ^ 0x23);
        assert_eq!(
            gen1_packet(4, Color(9, 8, 7))[..11],
            [0xFF, 0x92, 0x48, 5, 0, 4, 4, 9, 8, 7, 0xFF]
        );
    }

    #[test]
    fn gen3_key_and_checksum() {
        // OEM key with no token: pairs unfold until 8 bytes
        let k = gen3_key(AW3423DWF.key, &[]);
        assert_eq!(k[0], 0x3F ^ 0xF5);
        let token: Vec<u8> = (0..16).collect();
        let mixed = gen3_key(AW3423DWF.key, &token);
        assert_ne!(mixed, k);
        let c = gen3_colour(GEN3_ALL, Color(1, 2, 3), false);
        assert_eq!(c[74], 0x6E ^ 0x0B ^ 1 ^ 2 ^ 3 ^ 0x64);
        assert_eq!(gen3_login(&[1; 8])[65..73], [1; 8]);
    }
}
