//! Xiaohua keyboards (protocol learned from SignalRGB's Xiaohua_Keyboard_Controller.js).

use super::common::{open_with, positioned};
use crate::hid::{Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "xiaohua";
const REPORT: u8 = 0x06;
const SET_LEN: usize = 64;
const COLOR_LEN: usize = 65;
const COLORS: u8 = 0x0F;
const LEDS_PER_PACKET: usize = 18;

/// One wire layout: report id, the packets that enter direct mode and those that end a frame.
pub struct Zoned {
    pub report: u8,
    pub init: &'static [&'static [u8]],
    pub trailer: &'static [&'static [u8]],
    /// Wire slot of each LED, in `Device::leds` order.
    pub slots: &'static [u8],
}

/// Zone 0 colour packets: RGB by wire slot, 18 slots per packet, slot count up to the highest used.
pub fn zone_packets(report: u8, slots: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    let n = slots.iter().map(|&s| s as usize + 1).max().unwrap_or(0);
    let mut buf = vec![0u8; n * 3];
    for (&s, &c) in slots.iter().zip(colors) {
        buf[s as usize * 3..s as usize * 3 + 3].copy_from_slice(&rgb(c));
    }
    buf.chunks(LEDS_PER_PACKET * 3)
        .enumerate()
        .map(|(i, part)| {
            let mut p = vec![report, COLORS, 0x00, 0x00, i as u8, part.len() as u8];
            p.extend_from_slice(part);
            p
        })
        .collect()
}

impl Proto for Zoned {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for p in self.init {
            io.write(p, SET_LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in zone_packets(self.report, self.slots, colors) {
            io.write(&p, COLOR_LEN)?;
        }
        for p in self.trailer {
            io.write(p, COLOR_LEN)?;
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &vec![Color(0, 0, 0); self.slots.len()]);
    }
}

const INIT: &[&[u8]] = &[
    &[REPORT, 0x17, 0x00, 0x00, 0x00, 0x01, 0x01],
    &[
        REPORT, 0x08, 0x00, 0x00, 0x00, 0x0D, 0x02, 0x03, 0x03, 0xFF, 0, 0, 0, 0, 0, 0, 0x01,
    ],
];
/// An empty zone 1 must follow every frame.
const TRAILER: &[&[u8]] = &[
    &[REPORT, COLORS, 0x01, 0x00, 0x00, 0x36],
    &[REPORT, COLORS, 0x01, 0x00, 0x01, 0x2D],
];

/// The "60%" layout both known models (XS63RGB, XS61RGB) report.
const SLOTS_60: [u8; 61] = [
    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53,
    54, 55, 56, 58, 66, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 80, 88, 90, 91, 92, 93, 94, 95,
    96, 97, 98, 99, 102, 110, 111, 112, 116, 120, 121, 122, 123,
];
const POS_60: [(u8, u8); 61] = [
    (0, 0),
    (1, 0),
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
    (11, 1),
    (12, 1),
    (13, 1),
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
    (13, 2),
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
    (13, 3),
    (0, 4),
    (1, 4),
    (2, 4),
    (6, 4),
    (10, 4),
    (11, 4),
    (12, 4),
    (13, 4),
];

fn proto() -> Zoned {
    Zoned {
        report: REPORT,
        init: INIT,
        trailer: TRAILER,
        slots: &SLOTS_60,
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Keyboard,
        14,
        5,
        &POS_60,
    );
    open_with(api, info, FAMILY, dev, proto())
}

pub static DETECTORS: &[Detector] = &[Detector {
    name: "Xiaohua Keyboard (Husky Blizzard / Fantech MAXFIT61)",
    vid: 0x2E88,
    pid: 0xB30A,
    interface: Some(0),
    usage_page: Some(0xFF1B),
    usage: Some(0x0091),
    open,
}];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;

    #[test]
    fn frame_packs_slots_and_ends_with_empty_zone_one() {
        let mut io = Rec::default();
        let mut p = proto();
        let mut c = vec![Color(0, 0, 0); 61];
        c[0] = Color(1, 2, 3);
        c[60] = Color(7, 8, 9);
        p.frame(&mut io, &c).unwrap();
        assert_eq!(io.ops.len(), 7 + 2);
        let Op::Write(first) = &io.ops[0] else {
            panic!()
        };
        assert_eq!(first.len(), 65);
        assert_eq!(first[..6], [0x06, 0x0F, 0, 0, 0, 54]);
        let Op::Write(p1) = &io.ops[1] else { panic!() };
        assert_eq!(p1[6 + (22 - 18) * 3..][..3], [1, 2, 3]);
        let Op::Write(last) = &io.ops[6] else {
            panic!()
        };
        assert_eq!(last[..6], [0x06, 0x0F, 0, 0, 6, 16 * 3]);
        assert_eq!(last[6 + (123 - 108) * 3..][..3], [7, 8, 9]);
        assert_eq!(
            io.ops[8],
            Op::Write(crate::hid::padded(&[6, 0x0F, 1, 0, 1, 0x2D], 65))
        );
    }

    #[test]
    fn layout_tables_agree() {
        assert_eq!(SLOTS_60.len(), POS_60.len());
        assert!(POS_60.iter().all(|&(x, y)| x < 14 && y < 5));
    }
}
