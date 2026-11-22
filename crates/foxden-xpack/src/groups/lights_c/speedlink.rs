//! SpeedLink ORIOS mouse, mousepad and keyboard.

use super::common::{open_with, positioned};
use crate::hid::{Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "speedlink-orios";
const VID: u16 = 0x0C45;
const LEN: usize = 64;
const REPORT: u8 = 0x04;
const BEGIN: [u8; 4] = [REPORT, 0x01, 0x00, 0x01];
const END: [u8; 4] = [REPORT, 0x02, 0x00, 0x02];
const MOUSE_HEAD: u8 = 0x38;
const PAD_HEAD: u8 = 0x21;

/// Mouse and mousepad colour packet. LEDs sit 3 bytes apart with R, B, G at +0, +2, +4, so each
/// LED's green lands in the next slot's second byte; later LEDs overwrite earlier ones.
pub fn small_packet(head: u8, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0u8; 8 + colors.len() * 3 + 2];
    p[..5].copy_from_slice(&[REPORT, 0x00, 0x00, 0x12, head]);
    for (i, c) in colors.iter().enumerate() {
        let o = 8 + i * 3;
        p[o] = c.0;
        p[o + 2] = c.2;
        p[o + 4] = c.1;
    }
    p
}

struct Small {
    head: u8,
}

impl Proto for Small {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&BEGIN, LEN)?;
        io.write(&small_packet(self.head, colors), LEN)?;
        io.write(&END, LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let n = if self.head == MOUSE_HEAD {
            MOUSE_POS.len()
        } else {
            PAD_POS.len()
        };
        let _ = self.frame(io, &vec![Color(0, 0, 0); n]);
    }
}

const MOUSE_POS: [(u8, u8); 6] = [(3, 5), (3, 0), (3, 1), (3, 2), (3, 3), (3, 4)];
const PAD_POS: [(u8, u8); 10] = [
    (1, 0),
    (0, 0),
    (0, 1),
    (0, 2),
    (1, 2),
    (2, 2),
    (3, 2),
    (3, 1),
    (3, 0),
    (2, 0),
];

const KB_ROWS: usize = 7;
const KB_BYTES: usize = KB_ROWS * 56;
const ROW_B4: [u8; KB_ROWS] = [0x38, 0x38, 0x38, 0x38, 0x38, 0x38, 0x2A];
const ROW_B5: [u8; KB_ROWS] = [0x00, 0x38, 0x70, 0xA8, 0xE0, 0x18, 0x50];
const ROW_B6: [u8; KB_ROWS] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];

const KB_SLOTS: [u8; 105] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
    76, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 80, 81, 82,
    84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 97, 99, 101, 102, 103, 104, 105, 106, 107, 108,
    109, 110, 111, 113, 119, 120, 121, 123, 124,
];
const KB_POS: [(u8, u8); 105] = [
    (0, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (8, 0),
    (9, 0),
    (10, 0),
    (11, 0),
    (12, 0),
    (13, 0),
    (14, 0),
    (15, 0),
    (16, 0),
    (0, 1),
    (1, 1),
    (2, 1),
    (3, 1),
    (4, 1),
    (5, 1),
    (6, 1),
    (7, 1),
    (8, 1),
    (9, 1),
    (10, 1),
    (12, 1),
    (13, 1),
    (14, 1),
    (15, 1),
    (16, 1),
    (17, 1),
    (18, 1),
    (19, 1),
    (20, 1),
    (21, 1),
    (0, 2),
    (1, 2),
    (2, 2),
    (3, 2),
    (4, 2),
    (5, 2),
    (6, 2),
    (7, 2),
    (8, 2),
    (9, 2),
    (10, 2),
    (11, 2),
    (12, 2),
    (14, 2),
    (15, 2),
    (16, 2),
    (17, 2),
    (18, 2),
    (19, 2),
    (20, 2),
    (21, 2),
    (0, 3),
    (2, 3),
    (3, 3),
    (4, 3),
    (5, 3),
    (6, 3),
    (7, 3),
    (8, 3),
    (9, 3),
    (10, 3),
    (11, 3),
    (12, 3),
    (13, 3),
    (18, 3),
    (19, 3),
    (20, 3),
    (0, 4),
    (1, 4),
    (2, 4),
    (3, 4),
    (4, 4),
    (5, 4),
    (6, 4),
    (7, 4),
    (8, 4),
    (9, 4),
    (10, 4),
    (11, 4),
    (13, 4),
    (16, 4),
    (18, 4),
    (19, 4),
    (20, 4),
    (21, 4),
    (0, 5),
    (1, 5),
    (2, 5),
    (6, 5),
    (10, 5),
    (11, 5),
    (12, 5),
    (14, 5),
    (15, 5),
    (16, 5),
    (17, 5),
    (18, 5),
    (20, 5),
];

/// Seven row packets of 56 RGB bytes each, keys placed at their wire slot.
pub fn kb_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let mut buf = [0u8; KB_BYTES];
    for (&s, &c) in KB_SLOTS.iter().zip(colors) {
        buf[s as usize * 3..s as usize * 3 + 3].copy_from_slice(&rgb(c));
    }
    (0..KB_ROWS)
        .map(|r| {
            let mut p = vec![
                REPORT, 0x00, 0x00, 0x12, ROW_B4[r], ROW_B5[r], ROW_B6[r], 0x00,
            ];
            p.extend_from_slice(&buf[r * 56..(r + 1) * 56]);
            p
        })
        .collect()
}

struct Keyboard;

impl Proto for Keyboard {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in kb_packets(colors) {
            io.write(&p, LEN)?;
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); 105]);
    }
}

fn open_mouse(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Mouse,
        5,
        6,
        &MOUSE_POS,
    );
    open_with(api, info, FAMILY, dev, Small { head: MOUSE_HEAD })
}

fn open_pad(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Other,
        4,
        3,
        &PAD_POS,
    );
    open_with(api, info, FAMILY, dev, Small { head: PAD_HEAD })
}

fn open_kb(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Keyboard,
        22,
        6,
        &KB_POS,
    );
    open_with(api, info, FAMILY, dev, Keyboard)
}

const fn det(name: &'static str, pid: u16, open: foxden_hid::registry::Open) -> Detector {
    Detector {
        name,
        vid: VID,
        pid,
        interface: Some(1),
        usage_page: Some(0xFF1C),
        usage: Some(0x0092),
        open,
    }
}

pub static DETECTORS: &[Detector] = &[
    det("Speedlink ORIOS RGB Gaming Mouse", 0x5F01, open_mouse),
    det("Speedlink ORIOS RGB Gaming Mousepad", 0x763D, open_pad),
    det("Speedlink ORIOS RGB Opto-Mech KB", 0x763E, open_kb),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;

    #[test]
    fn small_packet_overlaps_like_the_plugin() {
        let p = small_packet(MOUSE_HEAD, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(p[..5], [4, 0, 0, 0x12, 0x38]);
        assert_eq!(p[8..16], [1, 0, 3, 4, 2, 6, 0, 5]);
    }

    #[test]
    fn small_frame_is_wrapped() {
        let mut io = Rec::default();
        Small { head: PAD_HEAD }
            .frame(&mut io, &[Color(9, 9, 9); 10])
            .unwrap();
        assert_eq!(io.ops.len(), 3);
        assert_eq!(io.ops[0], Op::Write(crate::hid::padded(&BEGIN, 64)));
        assert_eq!(io.ops[2], Op::Write(crate::hid::padded(&END, 64)));
    }

    #[test]
    fn keyboard_rows() {
        let mut c = vec![Color(0, 0, 0); 105];
        c[50] = Color(1, 2, 3);
        let p = kb_packets(&c);
        assert_eq!(p.len(), 7);
        assert!(p.iter().all(|r| r.len() == 64));
        assert_eq!(p[6][4..8], [0x2A, 0x50, 0x01, 0x00]);
        let at = 76 * 3 - 4 * 56;
        assert_eq!(p[4][8 + at..8 + at + 3], [1, 2, 3]);
        assert!(KB_POS.iter().all(|&(x, y)| x < 22 && y < 6));
    }
}
