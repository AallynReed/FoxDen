// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::klm::{Keymap, Layout, Overlay, Size, ins, new_row};
use super::{Driver, Lights, det, device, exchange, report, rgb};
use crate::registry::{Detector, Info};

const VID: u16 = 0x2516;
const I1: Option<i32> = Some(1);
const PAGE: Option<u16> = Some(0xFF00);
const USAGE: Option<u16> = Some(1);
const TIMEOUT: i32 = 50;

const PRO_L: u16 = 0x003B;
const PRO_L_WHITE: u16 = 0x0047;
const PRO_S: u16 = 0x003C;
const SK622W: u16 = 0x014B;
const SK622B: u16 = 0x0149;
const SK620: u16 = 0x0159;
const SK621: u16 = 0x0087;
const SK630: u16 = 0x0089;
const SK650: u16 = 0x008D;
const SK650_V2: u16 = 0x0123;
const SK652: u16 = 0x015D;
const SK653: u16 = 0x01AB;
const MK730: u16 = 0x008F;
const MK750: u16 = 0x0067;
const CK530: u16 = 0x009F;
const CK530_V2: u16 = 0x0147;
const CK550_V2: u16 = 0x0145;
const CK552_V2: u16 = 0x007F;

pub static DETECTORS: &[Detector] = &[
    det!(
        "Cooler Master MasterKeys Pro S",
        VID,
        PRO_S,
        I1,
        PAGE,
        USAGE,
        open_v1
    ),
    det!(
        "Cooler Master MasterKeys Pro L",
        VID,
        PRO_L,
        I1,
        PAGE,
        USAGE,
        open_v1
    ),
    det!(
        "Cooler Master MasterKeys Pro L White",
        VID,
        PRO_L_WHITE,
        I1,
        PAGE,
        USAGE,
        open_v1
    ),
    det!(
        "Cooler Master SK622 White",
        VID,
        SK622W,
        I1,
        PAGE,
        USAGE,
        open_v2
    ),
    det!(
        "Cooler Master SK622 Black",
        VID,
        SK622B,
        I1,
        PAGE,
        USAGE,
        open_v2
    ),
    det!("Cooler Master SK620", VID, SK620, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master SK621", VID, SK621, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master SK630", VID, SK630, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master SK650", VID, SK650, I1, PAGE, USAGE, open_v2),
    det!(
        "Cooler Master SK650",
        VID,
        SK650_V2,
        I1,
        PAGE,
        USAGE,
        open_v2
    ),
    det!("Cooler Master SK652", VID, SK652, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master SK653", VID, SK653, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master MK730", VID, MK730, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master MK750", VID, MK750, I1, PAGE, USAGE, open_v2),
    det!("Cooler Master CK530", VID, CK530, I1, PAGE, USAGE, open_v2),
    det!(
        "Cooler Master CK530 V2",
        VID,
        CK530_V2,
        I1,
        PAGE,
        USAGE,
        open_v2
    ),
    det!(
        "Cooler Master CK550 V2",
        VID,
        CK550_V2,
        I1,
        PAGE,
        USAGE,
        open_v2
    ),
    det!(
        "Cooler Master CK550 V1 / CK552",
        VID,
        CK552_V2,
        I1,
        PAGE,
        USAGE,
        open_v2
    ),
];

const fn overlay(
    size: Size,
    values: &'static [u32],
    edits: &'static [super::klm::Edit],
) -> Overlay {
    Overlay {
        size,
        values,
        edits,
    }
}

const MK_PRO_S: Overlay = overlay(Size::Tkl, MK_PRO_S_KEYMAP, &[]);
const MK_PRO_L: Overlay = overlay(
    Size::Full,
    MK_PRO_L_KEYMAP,
    &[
        ins(0, 17, 101),
        ins(0, 18, 109),
        ins(0, 19, 117),
        ins(0, 20, 119),
    ],
);
const SK622: Overlay = overlay(
    Size::Sixty,
    SK622_KEYMAP,
    &[
        new_row(0, 0, 7),
        ins(0, 1, 14),
        ins(0, 2, 21),
        ins(0, 3, 28),
        ins(0, 4, 35),
        ins(0, 5, 42),
        ins(0, 6, 49),
        ins(0, 7, 56),
        ins(0, 8, 63),
        ins(0, 9, 70),
        ins(0, 10, 77),
        ins(0, 11, 84),
        ins(0, 12, 91),
        ins(0, 13, 98),
        ins(0, 14, 105),
        ins(1, 0, 0),
        ins(2, 0, 1),
        ins(3, 0, 2),
        ins(4, 0, 3),
        ins(5, 0, 4),
        ins(1, 16, 112),
        ins(2, 16, 113),
        ins(3, 16, 114),
        ins(4, 16, 115),
        ins(5, 16, 116),
        new_row(6, 0, 6),
        ins(6, 1, 20),
        ins(6, 2, 27),
        ins(6, 3, 34),
        ins(6, 4, 41),
        ins(6, 5, 48),
        ins(6, 6, 55),
        ins(6, 7, 62),
        ins(6, 8, 69),
        ins(6, 9, 76),
        ins(6, 10, 83),
        ins(6, 11, 90),
        ins(6, 12, 97),
        ins(6, 13, 104),
        ins(6, 14, 111),
        ins(6, 14, 118),
    ],
);
const SK630_L: Overlay = overlay(Size::Tkl, SK630_KEYMAP, &[]);
const SK650_L: Overlay = overlay(Size::Full, SK650_KEYMAP, &[]);
const SK652_L: Overlay = overlay(Size::Full, SK652_KEYMAP, &[]);
const MK730_L: Overlay = overlay(
    Size::Tkl,
    MK730_KEYMAP,
    &[
        ins(1, 0, 1),
        ins(2, 0, 2),
        ins(3, 0, 3),
        ins(4, 0, 4),
        ins(1, 18, 141),
        ins(2, 18, 142),
        ins(3, 18, 143),
        ins(4, 18, 144),
        new_row(6, 1, 13),
        ins(6, 2, 20),
        ins(6, 3, 27),
        ins(6, 4, 34),
        ins(6, 5, 41),
        ins(6, 6, 55),
        ins(6, 7, 62),
        ins(6, 8, 69),
        ins(6, 10, 76),
        ins(6, 11, 90),
        ins(6, 12, 104),
        ins(6, 13, 111),
        ins(6, 14, 118),
        ins(6, 15, 125),
    ],
);
const MK750_L: Overlay = overlay(
    Size::Full,
    MK750_KEYMAP,
    &[
        ins(0, 17, 140),
        ins(0, 18, 147),
        ins(0, 19, 154),
        ins(0, 20, 161),
        ins(1, 0, 1),
        ins(2, 0, 2),
        ins(3, 0, 3),
        ins(4, 0, 4),
        ins(1, 22, 170),
        ins(2, 22, 171),
        ins(3, 22, 172),
        ins(4, 22, 173),
        new_row(6, 1, 20),
        ins(6, 2, 27),
        ins(6, 3, 34),
        ins(6, 4, 41),
        ins(6, 5, 55),
        ins(6, 6, 62),
        ins(6, 7, 69),
        ins(6, 8, 76),
        ins(6, 9, 83),
        ins(6, 10, 90),
        ins(6, 11, 104),
        ins(6, 12, 111),
        ins(6, 13, 118),
        ins(6, 14, 125),
        ins(6, 15, 132),
        ins(6, 16, 146),
        ins(6, 17, 153),
        ins(6, 18, 160),
    ],
);
const CK530_L: Overlay = overlay(Size::Tkl, CK530_KEYMAP, &[]);
const CK530_V2_L: Overlay = overlay(Size::Tkl, CK530_V2_KEYMAP, &[]);
const CK550_V2_L: Overlay = overlay(
    Size::Full,
    CK550_V2_KEYMAP,
    &[ins(0, 17, 120), ins(0, 18, 126), ins(0, 19, 132)],
);
const CK552_L: Overlay = overlay(Size::Full, CK552_KEYMAP, &[]);

/// cm_kb_device_list; the Pro L White and the SK653 reuse other keyboards' layouts, as in OpenRGB.
/// SignalRGB drives the SK620/SK621 like the SK622, and SK650 0x0123 with its layout and the SK652 start-up.
fn overlay_for(pid: u16) -> Option<&'static Overlay> {
    Some(match pid {
        PRO_S | PRO_L_WHITE => &MK_PRO_S,
        PRO_L => &MK_PRO_L,
        SK622W | SK622B | SK620 | SK621 => &SK622,
        SK630 => &SK630_L,
        SK650 | SK650_V2 => &SK650_L,
        SK652 | SK653 => &SK652_L,
        MK730 => &MK730_L,
        MK750 => &MK750_L,
        CK530 => &CK530_L,
        CK530_V2 => &CK530_V2_L,
        CK550_V2 => &CK550_V2_L,
        CK552_V2 => &CK552_L,
        _ => return None,
    })
}

pub type Report = [u8; 65];

/// SetControlMode
pub fn control_mode(mode: u8) -> Report {
    report(&[0x41, mode])
}

/// CMKeyboardV1Controller::SetMode for Direct. OpenRGB maps mode value 2 to Spectrum Cycle, so
/// selecting Direct sends that effect (speed 0) before manual control takes over.
pub fn v1_direct_mode() -> [Report; 3] {
    let mut effect = report(&[
        0x51, 0x2C, 0x00, 0x00, 0x02, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0,
    ]);
    effect[17..].fill(0xFF);
    [
        control_mode(0x01),
        report(&[0x51, 0x28, 0x00, 0x00, 0x02]),
        effect,
    ]
}

/// The colour map the controllers index by LED value.
fn color_map(values: &[u32], colors: &[Color]) -> Vec<Color> {
    let mut map = vec![Color::BLACK; 255];
    for (&v, &c) in values.iter().zip(colors) {
        if let Some(slot) = map.get_mut(v as usize) {
            *slot = c;
        }
    }
    map
}

/// CMKeyboardV1Controller::SetLeds: manual control, then 8 packets of 16 colours.
pub fn v1_frame(values: &[u32], colors: &[Color]) -> Vec<Report> {
    let map = color_map(values, colors);
    let mut out = vec![control_mode(0x02)];
    for i in 0..8 {
        let mut r = report(&[0xC0, 0x02, (i * 2) as u8, 0x00]);
        for j in 0..16 {
            r[5 + j * 3..8 + j * 3].copy_from_slice(&rgb(map[i * 16 + j]));
        }
        out.push(r);
    }
    out
}

/// CMKeyboardV2Controller::SetCustomMode and _SetEffectMode(DIRECT)
pub fn v2_direct_mode() -> [Report; 3] {
    [
        control_mode(0x80),
        report(&[0x52]),
        report(&[
            0x56, 0x81, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x88, 0x88,
            0x88, 0x88,
        ]),
    ]
}

fn magic(profile: u8) -> Report {
    report(&[
        0x56, 0x81, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, profile, 0x00, 0x00, 0x00, 0xBB, 0xBB,
        0xBB, 0xBB,
    ])
}

fn active_profile(profile: u8) -> Report {
    report(&[0x51, 0x00, 0x00, 0x00, profile])
}

fn magic_startup() -> [Report; 6] {
    [
        report(&[0x12]),
        report(&[0x12, 0x20]),
        report(&[0x12, 0x01]),
        report(&[0x12, 0x22]),
        report(&[0x42, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x01]),
        report(&[0x43, 0x00, 0x00, 0x00, 0x01]),
    ]
}

/// CMKeyboardV2Controller::Initialize, which every SetLeds repeats.
pub fn v2_initialize(pid: u16) -> Vec<Report> {
    match pid {
        SK622B | SK622W | SK620 | SK621 => {
            let mut v = magic_startup().to_vec();
            v.push(magic(0x09));
            v
        }
        SK630 | SK650 => vec![active_profile(0x0C)],
        SK652 | SK653 | SK650_V2 => {
            let mut v = magic_startup().to_vec();
            v.push(magic(0x0C));
            v
        }
        CK530 | CK530_V2 | MK730 => vec![active_profile(0x05), magic(0x0A)],
        CK550_V2 => vec![active_profile(0x05), magic(0x09)],
        CK552_V2 => vec![control_mode(0x80), magic(0x01)],
        _ => vec![report(&[
            0x56, 0x81, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0xBB, 0xBB,
            0xBB,
        ])],
    }
}

/// CMKeyboardV2Controller::SetLeds: a header packet with 40 colour bytes, nine of 60, then apply.
pub fn v2_frame(pid: u16, values: &[u32], colors: &[Color]) -> Vec<Report> {
    let linear: Vec<u8> = color_map(values, colors)
        .iter()
        .flat_map(|c| rgb(*c))
        .collect();
    let mut out = vec![control_mode(0x05)];
    out.extend(v2_initialize(pid));
    let mut first = report(&[
        0x56, 0x83, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0xC1, 0x00, 0x00, 0x00,
        0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
    ]);
    first[25..].copy_from_slice(&linear[..40]);
    out.push(first);
    for i in 1..10 {
        let mut r = report(&[0x56, 0x83, i as u8, 0x00]);
        let at = 40 + (i - 1) * 60;
        r[5..].copy_from_slice(&linear[at..at + 60]);
        out.push(r);
    }
    out.push(report(&[0x51, 0x28, 0x00, 0x00, 0xFF]));
    out
}

/// CMKeyboardV1Controller::_GetFirmwareVersion: a C string from byte 4.
pub fn v1_firmware(read: &[u8]) -> String {
    let tail = read.get(4..).unwrap_or(&[]);
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).into_owned()
}

/// CMKeyboardV2Controller::_GetFirmwareVersion: UTF-16 from byte 8.
pub fn v2_firmware(read: &[u8]) -> String {
    let units: Vec<u16> = read
        .get(8..)
        .unwrap_or(&[])
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// CMKeyboardV1Controller::GetKeyboardLayout
pub fn v1_layout(firmware: &str) -> Layout {
    match firmware.as_bytes().first() {
        Some(b'2') => Layout::Iso,
        Some(b'3') => Layout::Jis,
        _ => Layout::Ansi,
    }
}

/// CMKeyboardV2Controller::GetKeyboardLayout; OpenRGB's "serial" here is the product string.
pub fn v2_layout(firmware: &str, product: &str) -> Layout {
    let mut layout = match firmware.as_bytes().get(1) {
        Some(b'3') => Layout::Jis,
        Some(b'2') => Layout::Iso,
        _ => Layout::Ansi,
    };
    if product.contains("JP") {
        layout = Layout::Jis;
    }
    if product.contains("EU") {
        layout = Layout::Iso;
    }
    layout
}

struct Keyboard {
    hid: HidDevice,
    v2: bool,
    pid: u16,
    values: Vec<u32>,
}

impl Driver for Keyboard {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        let mut out = Vec::new();
        if first {
            if self.v2 {
                out.extend(v2_direct_mode());
            } else {
                out.extend(v1_direct_mode());
            }
        }
        if self.v2 {
            out.extend(v2_frame(self.pid, &self.values, colors));
        } else {
            out.extend(v1_frame(&self.values, colors));
        }
        for r in &out {
            exchange(&self.hid, r, TIMEOUT)?;
        }
        Ok(())
    }
}

fn open_v1(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, det, false)
}

fn open_v2(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, det, true)
}

fn open(api: &HidApi, info: &Info, det: &Detector, v2: bool) -> Result<Box<dyn Backend>, String> {
    let overlay = overlay_for(info.pid).ok_or("no layout for this keyboard")?;
    let hid = info.open(api)?;
    exchange(&hid, &control_mode(0x00), TIMEOUT)?;
    let layout = if v2 {
        let read = exchange(&hid, &report(&[0x12, 0x20]), TIMEOUT)?;
        v2_layout(&v2_firmware(&read), &info.product)
    } else {
        let read = exchange(&hid, &report(&[0x01, 0x02]), TIMEOUT)?;
        let firmware = v1_firmware(&read);
        if firmware.is_empty() {
            Layout::Ansi
        } else {
            v1_layout(&firmware)
        }
    };
    let keys = Keymap::build(layout, overlay);
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Keyboard,
        keys.positions(),
        [keys.cols as u32, keys.rows as u32],
    );
    let driver = Keyboard {
        hid,
        v2,
        pid: info.pid,
        values: keys.values(),
    };
    Ok(Lights::boxed("cooler master keyboard", dev, None, driver))
}

const MK_PRO_S_KEYMAP: &[u32] = &[
    96, 97, 98, 99, 104, 105, 106, 112, 113, 114, 67, 68, 69, 102, 103, 107, 0, 1, 8, 9, 16, 17,
    24, 25, 32, 33, 40, 41, 48, 49, 56, 57, 64, 2, 3, 10, 11, 18, 19, 26, 27, 34, 35, 42, 43, 50,
    51, 58, 59, 66, 4, 5, 12, 13, 20, 21, 28, 29, 36, 37, 44, 45, 89, 52, 6, 100, 7, 14, 15, 22,
    23, 30, 31, 38, 39, 46, 47, 61, 91, 90, 92, 93, 94, 60, 95, 54, 63, 62, 70,
];
const MK_PRO_L_KEYMAP: &[u32] = &[
    11, 22, 30, 25, 27, 7, 51, 57, 62, 86, 87, 83, 85, 79, 72, 0, 14, 15, 23, 31, 39, 38, 46, 47,
    55, 63, 71, 70, 54, 81, 3, 1, 2, 100, 108, 116, 118, 9, 8, 16, 24, 32, 33, 41, 40, 48, 56, 64,
    65, 49, 82, 94, 92, 88, 96, 104, 112, 110, 17, 10, 18, 26, 34, 35, 43, 42, 50, 58, 66, 67, 68,
    84, 97, 105, 113, 73, 19, 12, 20, 28, 36, 37, 45, 44, 52, 60, 69, 74, 80, 98, 106, 114, 111, 6,
    90, 75, 91, 77, 78, 61, 4, 95, 93, 5, 107, 115,
];
const SK622_KEYMAP: &[u32] = &[
    8, 15, 22, 29, 36, 43, 50, 57, 64, 71, 78, 85, 92, 106, 9, 23, 30, 37, 44, 51, 58, 65, 72, 79,
    86, 93, 100, 10, 24, 31, 38, 45, 52, 59, 66, 73, 80, 87, 94, 101, 108, 11, 18, 25, 32, 39, 46,
    53, 60, 67, 74, 81, 88, 95, 102, 109, 12, 19, 26, 54, 82, 89, 96, 103, 110,
];
const SK630_KEYMAP: &[u32] = &[
    9, 33, 41, 49, 57, 73, 81, 89, 97, 105, 113, 121, 129, 137, 145, 153, 10, 26, 34, 42, 50, 58,
    66, 74, 82, 90, 98, 106, 114, 130, 138, 146, 154, 11, 27, 35, 43, 51, 59, 67, 75, 83, 91, 99,
    107, 115, 131, 139, 147, 155, 12, 28, 36, 44, 52, 60, 68, 76, 84, 92, 100, 108, 116, 132, 13,
    21, 29, 37, 45, 53, 61, 69, 77, 85, 93, 101, 133, 149, 14, 22, 30, 62, 94, 102, 110, 134, 142,
    150, 158,
];
const SK650_KEYMAP: &[u32] = &[
    0, 24, 32, 40, 48, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 1, 17, 25, 33, 41, 49, 57,
    65, 73, 81, 89, 97, 105, 121, 129, 137, 145, 153, 161, 169, 177, 2, 18, 26, 34, 42, 50, 58, 66,
    74, 82, 90, 98, 106, 122, 130, 138, 146, 154, 162, 170, 178, 3, 19, 27, 35, 43, 51, 59, 67, 75,
    83, 91, 99, 107, 123, 155, 163, 171, 4, 20, 28, 36, 44, 52, 60, 68, 76, 84, 92, 100, 124, 140,
    156, 164, 172, 180, 5, 13, 21, 53, 85, 93, 101, 125, 133, 141, 149, 165, 173,
];
const SK652_KEYMAP: &[u32] = &[
    9, 33, 41, 49, 57, 73, 81, 89, 97, 105, 113, 121, 129, 137, 145, 153, 10, 26, 34, 42, 50, 58,
    66, 74, 82, 90, 98, 106, 114, 130, 138, 146, 154, 162, 170, 178, 186, 11, 27, 35, 43, 51, 59,
    67, 75, 83, 91, 99, 107, 115, 131, 139, 147, 155, 163, 171, 179, 187, 12, 28, 36, 44, 52, 60,
    68, 76, 84, 92, 100, 108, 116, 132, 164, 172, 180, 13, 21, 29, 37, 45, 53, 61, 69, 77, 85, 93,
    101, 133, 149, 165, 173, 181, 189, 14, 22, 30, 62, 94, 102, 110, 134, 142, 150, 158, 174, 182,
];
const MK730_KEYMAP: &[u32] = &[
    7, 28, 35, 42, 49, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 8, 22, 29, 36, 43, 50, 57,
    64, 71, 78, 85, 92, 99, 113, 120, 127, 134, 9, 23, 30, 37, 44, 51, 58, 65, 72, 79, 86, 93, 100,
    114, 121, 128, 135, 10, 24, 31, 38, 45, 52, 59, 66, 73, 80, 87, 94, 108, 115, 11, 18, 25, 32,
    39, 46, 53, 60, 67, 74, 81, 88, 116, 130, 12, 19, 26, 54, 82, 89, 96, 117, 124, 131, 138,
];
const MK750_KEYMAP: &[u32] = &[
    7, 28, 35, 42, 49, 63, 70, 77, 84, 91, 98, 105, 112, 119, 136, 133, 8, 22, 29, 36, 43, 50, 57,
    64, 71, 78, 85, 92, 99, 113, 120, 127, 134, 0, 148, 155, 162, 9, 23, 30, 37, 44, 51, 58, 65,
    72, 79, 86, 93, 100, 114, 121, 128, 135, 142, 149, 156, 163, 10, 24, 31, 38, 45, 52, 59, 66,
    73, 80, 87, 94, 108, 115, 143, 150, 157, 11, 18, 25, 32, 39, 46, 53, 60, 67, 74, 81, 88, 116,
    130, 144, 151, 158, 165, 12, 19, 26, 54, 82, 89, 96, 117, 124, 131, 138, 152, 159,
];
const CK530_KEYMAP: &[u32] = &[
    6, 21, 27, 34, 41, 55, 62, 69, 76, 83, 90, 97, 104, 111, 118, 125, 7, 15, 22, 28, 35, 42, 49,
    56, 63, 70, 77, 84, 91, 105, 112, 119, 126, 8, 16, 23, 29, 36, 43, 50, 57, 64, 71, 78, 85, 92,
    106, 113, 120, 127, 9, 17, 24, 30, 37, 44, 51, 58, 65, 72, 79, 86, 93, 107, 10, 18, 25, 31, 38,
    45, 52, 59, 66, 73, 80, 87, 108, 122, 11, 12, 19, 46, 74, 81, 88, 109, 116, 123, 130,
];
const CK530_V2_KEYMAP: &[u32] = &[
    7, 28, 35, 42, 49, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 8, 22, 29, 36, 43, 50, 57,
    64, 71, 78, 85, 92, 99, 113, 120, 127, 134, 9, 23, 30, 37, 44, 51, 58, 65, 72, 79, 86, 93, 100,
    114, 121, 128, 135, 10, 24, 31, 38, 45, 52, 59, 66, 73, 80, 87, 94, 108, 115, 11, 18, 25, 32,
    39, 46, 53, 60, 67, 74, 88, 81, 116, 130, 12, 19, 26, 54, 82, 89, 96, 117, 124, 131, 138,
];
const CK550_V2_KEYMAP: &[u32] = &[
    0, 18, 24, 30, 36, 48, 54, 60, 66, 72, 78, 84, 90, 96, 102, 108, 1, 13, 19, 25, 31, 37, 43, 49,
    55, 61, 67, 73, 79, 91, 97, 103, 109, 115, 121, 127, 133, 2, 14, 20, 26, 32, 38, 44, 50, 56,
    62, 68, 74, 80, 92, 98, 104, 110, 116, 122, 128, 134, 3, 15, 21, 27, 33, 39, 45, 51, 57, 63,
    69, 75, 87, 93, 117, 123, 129, 4, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 94, 106, 118,
    124, 130, 136, 5, 11, 17, 41, 65, 71, 77, 95, 101, 107, 113, 125, 131,
];
const CK552_KEYMAP: &[u32] = &[
    0, 18, 24, 30, 36, 48, 54, 60, 66, 72, 78, 84, 90, 96, 102, 108, 1, 13, 19, 25, 31, 37, 43, 49,
    55, 61, 67, 73, 79, 91, 97, 103, 109, 115, 121, 127, 133, 2, 14, 20, 26, 32, 38, 44, 50, 56,
    62, 68, 74, 80, 92, 98, 104, 110, 116, 122, 128, 134, 3, 15, 21, 27, 33, 39, 45, 51, 57, 63,
    69, 75, 81, 93, 117, 123, 129, 4, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 94, 106, 118,
    124, 130, 136, 5, 11, 17, 41, 65, 71, 77, 95, 101, 107, 113, 125, 131,
];

#[cfg(test)]
mod tests {
    use super::*;

    // CMKeyboardAbstractController::SetControlMode
    #[test]
    fn control_mode_bytes() {
        assert_eq!(&control_mode(0x02)[..4], &[0x00, 0x41, 0x02, 0x00]);
    }

    // CMKeyboardV1Controller::SetMode(Direct) through SetEffect
    #[test]
    fn v1_direct_mode_bytes() {
        let [custom, active, effect] = v1_direct_mode();
        assert_eq!(custom, report(&[0x41, 0x01]));
        assert_eq!(active, report(&[0x51, 0x28, 0x00, 0x00, 0x02]));
        assert_eq!(
            &effect[..17],
            &[
                0x00, 0x51, 0x2C, 0x00, 0x00, 0x02, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0
            ]
        );
        assert!(effect[17..].iter().all(|&b| b == 0xFF));
    }

    // CMKeyboardV1Controller::SetLeds
    #[test]
    fn v1_frame_bytes() {
        let f = v1_frame(
            &[96, 0, 17],
            &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)],
        );
        assert_eq!(f.len(), 9);
        assert_eq!(f[0], control_mode(0x02));
        assert_eq!(&f[1][..8], &[0x00, 0xC0, 0x02, 0x00, 0x00, 4, 5, 6]);
        assert_eq!(&f[2][..5], &[0x00, 0xC0, 0x02, 0x02, 0x00]);
        assert_eq!(&f[2][8..11], &[7, 8, 9]);
        assert_eq!(&f[7][..8], &[0x00, 0xC0, 0x02, 0x0C, 0x00, 1, 2, 3]);
        assert_eq!(&f[8][..4], &[0x00, 0xC0, 0x02, 0x0E]);
        assert!(f[8][53..].iter().all(|&b| b == 0));
    }

    // CMKeyboardV2Controller::SetCustomMode and _SetEffectMode(DIRECT)
    #[test]
    fn v2_direct_mode_bytes() {
        let [a, b, c] = v2_direct_mode();
        assert_eq!(a, report(&[0x41, 0x80]));
        assert_eq!(&b[..3], &[0x00, 0x52, 0x00]);
        assert_eq!(
            &c[..18],
            &[
                0x00, 0x56, 0x81, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x88,
                0x88, 0x88, 0x88, 0x00
            ]
        );
    }

    // CMKeyboardV2Controller::Initialize per keyboard
    #[test]
    fn v2_initialize_sequences() {
        assert_eq!(
            v2_initialize(CK550_V2),
            vec![active_profile(0x05), magic(0x09)]
        );
        assert_eq!(v2_initialize(SK630), vec![report(&[0x51, 0, 0, 0, 0x0C])]);
        let sk622 = v2_initialize(SK622B);
        assert_eq!(sk622.len(), 7);
        assert_eq!(
            &sk622[4][..10],
            &[0x00, 0x42, 0, 0, 0, 0x01, 0, 0, 0x01, 0x01]
        );
        assert_eq!(
            &sk622[6][..18],
            &[
                0x00, 0x56, 0x81, 0, 0, 1, 0, 0, 0, 0x09, 0, 0, 0, 0xBB, 0xBB, 0xBB, 0xBB, 0
            ]
        );
        assert_eq!(
            v2_initialize(CK552_V2),
            vec![control_mode(0x80), magic(0x01)]
        );
        let mk750 = v2_initialize(MK750);
        assert_eq!(
            &mk750[0][..17],
            &[
                0x00, 0x56, 0x81, 0, 0, 1, 0, 0, 0, 0x0A, 0, 0, 0, 0xBB, 0xBB, 0xBB, 0x00
            ]
        );
    }

    // CMKeyboardV2Controller::SetLeds
    #[test]
    fn v2_frame_bytes() {
        let f = v2_frame(
            CK550_V2,
            &[0, 13, 254],
            &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)],
        );
        assert_eq!(f.len(), 1 + 2 + 1 + 9 + 1);
        assert_eq!(f[0], control_mode(0x05));
        let first = &f[3];
        assert_eq!(
            &first[..25],
            &[
                0x00, 0x56, 0x83, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0xC1, 0x00,
                0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00
            ]
        );
        assert_eq!(&first[25..28], &[1, 2, 3]);
        // value 13 starts at linear byte 39, the last of the header packet
        assert_eq!(first[64], 4);
        assert_eq!(&f[4][..7], &[0x00, 0x56, 0x83, 0x01, 0x00, 5, 6]);
        assert_eq!(&f[12][..4], &[0x00, 0x56, 0x83, 0x09]);
        assert_eq!(f[13], report(&[0x51, 0x28, 0x00, 0x00, 0xFF]));
        // value 254 sits past the 580 bytes SetLeds sends
        assert!(f.iter().all(|r| !r.windows(3).any(|w| w == [7, 8, 9])));
    }

    // CMKeyboardV1Controller::_GetFirmwareVersion and GetKeyboardLayout
    #[test]
    fn v1_firmware_layout() {
        let read = [0x01, 0x02, 0x00, 0x00, b'2', b'.', b'0', 0x00, b'x'];
        assert_eq!(v1_firmware(&read), "2.0");
        assert_eq!(v1_layout("2.0"), Layout::Iso);
        assert_eq!(v1_layout("3.1"), Layout::Jis);
        assert_eq!(v1_layout("1.0"), Layout::Ansi);
    }

    // CMKeyboardV2Controller::_GetFirmwareVersion and GetKeyboardLayout
    #[test]
    fn v2_firmware_layout() {
        let mut read = vec![0u8; 8];
        for u in "V2.01".encode_utf16() {
            read.extend(u.to_le_bytes());
        }
        read.extend([0, 0, b'z', 0]);
        assert_eq!(v2_firmware(&read), "V2.01");
        assert_eq!(v2_layout("V2.01", "CK550"), Layout::Iso);
        assert_eq!(v2_layout("V1.00", "SK622 JP"), Layout::Jis);
        assert_eq!(v2_layout("V3.00", "SK622 EU"), Layout::Iso);
        assert_eq!(v2_layout("", "SK650"), Layout::Ansi);
    }

    // RGBController_CMKeyboardController::SetupZones key counts through KeyboardLayoutManager
    #[test]
    fn key_counts() {
        let n = |pid| {
            Keymap::build(Layout::Ansi, overlay_for(pid).unwrap())
                .keys
                .len()
        };
        assert_eq!(n(PRO_S), 87);
        assert_eq!(n(PRO_L), 108);
        assert_eq!(n(SK622B), 101);
        assert_eq!(n(MK730), 108);
        assert_eq!(n(MK750), 133);
        assert_eq!(n(CK550_V2), 107);
        assert_eq!(n(CK552_V2), 104);
        let pro_s = Keymap::build(Layout::Ansi, &MK_PRO_S);
        assert_eq!(&pro_s.values()[..3], &[96, 97, 98]);
    }

    #[test]
    fn keymaps_fill_their_base_layouts() {
        assert_eq!(MK_PRO_S_KEYMAP.len(), 89);

        assert_eq!(MK_PRO_L_KEYMAP.len(), 106);
        assert_eq!(CK550_V2_KEYMAP.len(), 106);
    }
}
