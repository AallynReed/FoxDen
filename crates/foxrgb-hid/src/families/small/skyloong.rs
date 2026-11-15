//! Ported from OpenRGB Controllers/SkyloongController (GPL-2.0-or-later).

use super::common::*;
use super::klm::{FULL, Klm, Layout, Op as KeyOp, k};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "skyloong";
const PING: u8 = 0x0C;
const MODE: u8 = 0x0B;
const LE_DEFINE: u8 = 0x1A;
const MODE_ONLINE: u8 = 0x05;
const LE_DEFINE_SET: u8 = 0x01;
const LE_DEFINE_SAVE: u8 = 0x02;
const TOTAL_LED_BYTES: usize = 528;
const CHUNK: usize = 56;
const BRIGHTNESS_MAX: u8 = 127;

const OFFSETS: [u32; 106] = [
    0, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56,
    58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 84, 85,
    86, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 102, 104, 106, 107, 108, 109, 110, 111,
    112, 116, 120, 121, 122, 124, 125, 126, 127, 128, 130,
];

fn keyboard() -> Klm {
    let mut kb = Klm::new(Layout::AnsiQwerty, FULL, &OFFSETS, &[]);
    kb.change_keys(&[
        k(4, 12, 0, "Right Shift", KeyOp::RemoveShiftLeft),
        k(4, 14, 0, "", KeyOp::InsertShiftRight),
        k(5, 4, 114, "Left Space", KeyOp::SwapOnly),
        k(5, 8, 118, "Right Space", KeyOp::SwapOnly),
    ]);
    kb
}

pub fn crc16_ccitt_false(buf: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in buf {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn sealed(mut buf: [u8; 65]) -> [u8; 65] {
    let crc = crc16_ccitt_false(&buf);
    buf[7] = crc as u8;
    buf[8] = (crc >> 8) as u8;
    buf
}

pub fn command(cmd: u8, sub: u8) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[1] = cmd;
    b[2] = sub;
    sealed(b)
}

pub fn le_define(address: usize, data: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[1] = LE_DEFINE;
    b[2] = LE_DEFINE_SET;
    let header = (address as u32).wrapping_add((data.len() as u32) << 24);
    b[3..7].copy_from_slice(&header.to_le_bytes());
    b[9..9 + data.len()].copy_from_slice(data);
    sealed(b)
}

pub fn colour_packets(values: &[u32], colors: &[Color], brightness: u8) -> Vec<[u8; 65]> {
    let mut le = [0u8; TOTAL_LED_BYTES];
    for (&v, c) in values.iter().zip(colors) {
        let i = v as usize * 4;
        le[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, brightness]);
    }
    let mut out: Vec<[u8; 65]> = le
        .chunks(CHUNK)
        .enumerate()
        .map(|(n, chunk)| le_define(n * CHUNK, chunk))
        .collect();
    out.push(command(LE_DEFINE, LE_DEFINE_SAVE));
    out
}

struct Skyloong {
    values: Vec<u32>,
}

impl Proto for Skyloong {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&command(PING, 0))?;
        io.write(&command(MODE, MODE_ONLINE))?;
        io.write(&command(PING, 0))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        colour_packets(&self.values, colors, BRIGHTNESS_MAX)
            .iter()
            .try_for_each(|p| io.write(p))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let kb = keyboard();
    let proto = Skyloong {
        values: kb.values(),
    };
    boxed(api, info, FAMILY, kb.device(info, FAMILY, d), proto)
}

pub static DETECTORS: &[Detector] = &[det("Skyloong GK104 Pro", 0x1EA7, 0x0907, 1, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout_has_both_space_halves() {
        let kb = keyboard();
        assert_eq!(kb.keys.len(), 106);
        let v = kb.values();
        assert!(v.contains(&114) && v.contains(&118));
        assert_eq!(
            kb.keys
                .iter()
                .find(|k| k.name == "Right Shift")
                .unwrap()
                .col,
            12
        );
        assert_eq!(
            kb.keys.iter().find(|k| k.name == "Up Arrow").unwrap().col,
            15
        );
    }

    // CRC16-CCITT-FALSE check value from the standard test vector
    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
    }

    // Constructor pings and goes online, then SendColorPacket writes 10 chunks and saves
    #[test]
    fn handshake_then_chunks() {
        let kb = keyboard();
        let n = kb.keys.len();
        let mut a = vec![Color::BLACK; n];
        a[0] = Color(1, 2, 3); // Escape, value 0
        let [first, _, _] = pushes(
            Skyloong {
                values: kb.values(),
            },
            n,
            vec![],
            &a,
            &[],
        );
        assert_eq!(first.len(), 3 + 10 + 1);
        let ping = command(PING, 0);
        assert_eq!(ping[..3], [0, 0x0C, 0]);
        assert_eq!(first[0], w(ping.to_vec()));
        assert_eq!(first[1], w(command(0x0B, 0x05).to_vec()));
        let Op::W(c0) = &first[3] else { panic!() };
        assert_eq!(c0[..7], [0, 0x1A, 0x01, 0, 0, 0, 56]);
        assert_eq!(c0[9..13], [1, 2, 3, 127]);
        let mut zeroed = c0.clone();
        zeroed[7] = 0;
        zeroed[8] = 0;
        let crc = crc16_ccitt_false(&zeroed);
        assert_eq!(c0[7..9], [crc as u8, (crc >> 8) as u8]);
        let Op::W(last) = &first[12] else { panic!() };
        assert_eq!(last[..7], [0, 0x1A, 0x01, 0xF8, 0x01, 0, 24]);
        assert_eq!(first[13], w(command(0x1A, 0x02).to_vec()));
    }
}
