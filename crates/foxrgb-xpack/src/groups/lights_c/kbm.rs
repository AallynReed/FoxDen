//! KBM! Gaming keyboards (protocol learned from SignalRGB's KBM_Keyboard_Controller.js).

use super::common::{open_with, positioned};
use super::xiaohua::Zoned;
use foxrgb_core::{Backend, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "kbm-gaming";
const REPORT: u8 = 0x01;

const INIT: &[&[u8]] = &[
    &[REPORT, 0x07, 0x00, 0x00, 0x00, 0x0E, 0x00, 0x04, 0x03, 0xFF],
    &[REPORT, 0x17, 0x00, 0x00, 0x00, 0x01, 0x01],
    &[
        REPORT, 0x08, 0x00, 0x00, 0x00, 0x0D, 0x02, 0x03, 0x03, 0xFF, 0, 0, 0, 0, 0, 0, 0x01,
    ],
];
const APPLY: &[&[u8]] = &[&[REPORT, 0x0F, 0x01, 0x00, 0x02, 0x1B]];

struct Layout {
    pid: u16,
    w: u32,
    slots: &'static [u8],
    pos: &'static [(u8, u8)],
}

const TG600_SLOTS: [u8; 63] = [
    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53,
    54, 55, 56, 66, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 88, 89, 90, 91, 92, 93, 94,
    95, 96, 97, 98, 99, 100, 101, 110, 111, 112, 116, 119, 120, 121, 122,
];
const TG600_POS: [(u8, u8); 63] = [
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
    (13, 2),
    (0, 3),
    (1, 3),
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
    (0, 4),
    (1, 4),
    (2, 4),
    (6, 4),
    (10, 4),
    (11, 4),
    (12, 4),
    (13, 4),
];
const TG700_SLOTS: [u8; 69] = [
    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36, 38, 44, 45, 46, 47, 48, 49, 50, 51, 52,
    53, 54, 55, 56, 60, 66, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 82, 88, 89, 90, 91,
    92, 93, 94, 95, 96, 97, 98, 99, 101, 103, 104, 110, 111, 112, 116, 119, 120, 121, 122, 125,
    126,
];
const TG700_POS: [(u8, u8); 69] = [
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
    (14, 0),
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
    (14, 1),
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
    (13, 2),
    (14, 2),
    (0, 3),
    (1, 3),
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
    (14, 3),
    (0, 4),
    (1, 4),
    (2, 4),
    (6, 4),
    (9, 4),
    (10, 4),
    (11, 4),
    (12, 4),
    (13, 4),
    (14, 4),
];

const LAYOUTS: [Layout; 2] = [
    Layout {
        pid: 0xE987,
        w: 15,
        slots: &TG700_SLOTS,
        pos: &TG700_POS,
    },
    Layout {
        pid: 0xE989,
        w: 14,
        slots: &TG600_SLOTS,
        pos: &TG600_POS,
    },
];

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let l = LAYOUTS
        .iter()
        .find(|l| l.pid == info.pid)
        .ok_or("unknown KBM model")?;
    let dev = positioned(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Keyboard,
        l.w,
        5,
        l.pos,
    );
    let proto = Zoned {
        report: REPORT,
        init: INIT,
        trailer: APPLY,
        slots: l.slots,
    };
    open_with(api, info, FAMILY, dev, proto)
}

const fn det(name: &'static str, pid: u16) -> Detector {
    Detector {
        name,
        vid: 0x4A4C,
        pid,
        interface: Some(2),
        usage_page: Some(0xFF1B),
        usage: Some(0x0091),
        open,
    }
}

pub static DETECTORS: &[Detector] = &[
    det("KBM! TG700 Preto", 0xE987),
    det("KBM! TG600 Branco", 0xE989),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;
    use crate::hid::{Proto, padded};
    use foxrgb_core::Color;

    #[test]
    fn start_then_frame_then_apply() {
        let l = &LAYOUTS[0];
        let mut p = Zoned {
            report: REPORT,
            init: INIT,
            trailer: APPLY,
            slots: l.slots,
        };
        let mut io = Rec::default();
        p.start(&mut io).unwrap();
        assert_eq!(io.ops[0], Op::Write(padded(INIT[0], 64)));
        io.ops.clear();
        let mut c = vec![Color(0, 0, 0); 69];
        c[68] = Color(4, 5, 6);
        p.frame(&mut io, &c).unwrap();
        assert_eq!(io.ops.len(), 8 + 1);
        let Op::Write(last) = &io.ops[7] else {
            panic!()
        };
        assert_eq!(last[..6], [1, 0x0F, 0, 0, 7, 3]);
        assert_eq!(last[6..9], [4, 5, 6]);
        assert_eq!(io.ops[8], Op::Write(padded(&[1, 0x0F, 1, 0, 2, 0x1B], 65)));
    }

    #[test]
    fn layout_tables_agree() {
        for l in &LAYOUTS {
            assert_eq!(l.slots.len(), l.pos.len());
            assert!(l.pos.iter().all(|&(x, y)| (x as u32) < l.w && y < 5));
        }
    }
}
