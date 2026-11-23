// SPDX-License-Identifier: GPL-2.0-or-later

use std::thread::sleep;
use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::klm::{Edit, Keymap, Layout, Overlay, Size, ins, new_row, swap, unused};
use super::{Driver, Lights, det, device, line, read, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1B1C;
const I1: Option<i32> = Some(1);
const PAGE: Option<u16> = Some(0xFF42);
const NA: u32 = u32::MAX;
const TIMEOUT: i32 = 50;
const TIMEOUT_SHORT: i32 = 3;
const WRITE_PACE: Duration = Duration::from_millis(2);
/// The keepalive thread resends the frame when nothing was sent for this long.
const UPDATE_PERIOD: Duration = Duration::from_millis(30000);
const WIRED: u8 = 0x08;
const WIRELESS: u8 = 0x09;
const LIGHT_CTRL1: u8 = 1;
const LIGHT_CTRL2: u8 = 0x22;
const LIGHT_MONO: u8 = 0x10;
const MODE_HW: u8 = 0x01;
const MODE_SW: u8 = 0x02;
const DIRECT: u16 = 0x0012;

pub static DETECTORS: &[Detector] = &[
    det!("Corsair K55 RGB PRO", VID, 0x1BA4, I1, PAGE, None, open_sw),
    det!(
        "Corsair K57 RGB (Wired)",
        VID,
        0x1B6E,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!("Corsair K60 RGB PRO", VID, 0x1BA0, I1, PAGE, None, open_sw),
    det!("Corsair K60 PRO", VID, 0x1B83, I1, PAGE, None, open_sw),
    det!(
        "Corsair K60 RGB PRO SE",
        VID,
        0x1B8D,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair K60 RGB PRO Low Profile",
        VID,
        0x1BAD,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair K60 RGB PRO TKL Black",
        VID,
        0x1BC7,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair K60 RGB PRO TKL White",
        VID,
        0x1BED,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!("Corsair K65 Mini", VID, 0x1BAF, I1, PAGE, None, open_hw),
    det!("Corsair K70 Core RGB", VID, 0x1BFD, I1, PAGE, None, open_hw),
    det!(
        "Corsair K70 Core RGB TKL",
        VID,
        0x2B01,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!("Corsair K70 RGB PRO", VID, 0x1BC4, I1, PAGE, None, open_hw),
    det!(
        "Corsair K70 RGB PRO V2",
        VID,
        0x1BB3,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!("Corsair K70 RGB TKL", VID, 0x1B73, I1, PAGE, None, open_hw),
    det!(
        "Corsair K70 RGB TKL Champion Series",
        VID,
        0x1BB9,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair K95 RGB PLATINUM XT",
        VID,
        0x1B89,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair K100 RGB Optical",
        VID,
        0x1B7C,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair K100 RGB Optical",
        VID,
        0x1BC5,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!("Corsair K100 MX Red", VID, 0x1B7D, I1, PAGE, None, open_hw),
    det!(
        "Corsair Dark Core RGB SE (Wired)",
        VID,
        0x1B4B,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair Dark Core RGB Pro SE (Wired)",
        VID,
        0x1B7E,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair Harpoon Wireless (Wired)",
        VID,
        0x1B5E,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair Ironclaw Wireless (Wired)",
        VID,
        0x1B4C,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair Ironclaw Wireless SE (Wired)",
        VID,
        0x2B32,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair Ironclaw Wireless SE",
        VID,
        0x2B00,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!("Corsair Katar Pro", VID, 0x1B93, I1, PAGE, None, open_hw),
    det!("Corsair Katar Pro V2", VID, 0x1BBA, I1, PAGE, None, open_hw),
    det!("Corsair Katar Pro XT", VID, 0x1BAC, I1, PAGE, None, open_hw),
    det!("Corsair M55 RGB PRO", VID, 0x1B70, I1, PAGE, None, open_sw),
    det!(
        "Corsair M65 RGB Ultra Wired",
        VID,
        0x1B9E,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair M65 RGB Ultra Wireless (Wired)",
        VID,
        0x1BB5,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair M75 Gaming Mouse",
        VID,
        0x1BF0,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!("Corsair M75 Wireless", VID, 0x1BDC, I1, PAGE, None, open_hw),
    det!(
        "Corsair M75 Wireless (Wired)",
        VID,
        0x1BF1,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair Scimitar Elite RGB",
        VID,
        0x1BE3,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair Slipstream Wireless Receiver HW",
        VID,
        0x1BA6,
        I1,
        PAGE,
        None,
        open_hw
    ),
    det!(
        "Corsair Slipstream Wireless Receiver SW",
        VID,
        0x1B65,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!(
        "Corsair Slipstream Wireless Receiver HW",
        VID,
        0x1B66,
        I1,
        PAGE,
        None,
        open_sw
    ),
    det!("Corsair MM700", VID, 0x1B9B, I1, PAGE, None, open_sw),
    det!("Corsair MM700 3XL", VID, 0x1BC9, I1, PAGE, None, open_sw),
];

const RECEIVERS: [u16; 5] = [0x1BA6, 0x1B66, 0x1B65, 0x1BDC, 0x2B00];
const K57_WIRED: u16 = 0x1B6E;
/// Single-colour boards: one intensity per LED instead of three planes.
const MONO: [u16; 1] = [0x1B83];
/// These keep lighting endpoint 2 without probing it.
const PINNED: [u16; 4] = [0x1B7C, 0x1BC5, 0x1B7D, 0x1BAF];

#[rustfmt::skip]
const TKL_VALUES: &[u32] = &[
    41, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72,
    53, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46, 42, 73, 74, 75,
    43, 20, 26, 8, 21, 23, 28, 24, 12, 18, 19, 47, 48, 49, 76, 77, 78,
    57, 4, 22, 7, 9, 10, 11, 13, 14, 15, 51, 52, 50, 40,
    106, 100, 29, 27, 6, 25, 5, 17, 16, 54, 55, 56, 110, 82,
    105, 108, 107, 44, 111, 122, 101, 109, 80, 81, 79,
];

#[rustfmt::skip]
const FULL_VALUES: &[u32] = &[
    41, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72,
    53, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46, 42, 73, 74, 75, 83, 84, 85, 86,
    43, 20, 26, 8, 21, 23, 28, 24, 12, 18, 19, 47, 48, 49, 76, 77, 78, 95, 96, 97, 87,
    57, 4, 22, 7, 9, 10, 11, 13, 14, 15, 51, 52, 50, 40, 92, 93, 94,
    106, 100, 29, 27, 6, 25, 5, 17, 16, 54, 55, 56, 110, 82, 89, 90, 91, 88,
    105, 108, 107, 44, 111, 122, 101, 109, 80, 81, 79, 98, 99,
];

#[rustfmt::skip]
const K65_MINI_VALUES: &[u32] = &[
    41, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46, 42,
    43, 20, 26, 8, 21, 23, 28, 24, 12, 18, 19, 47, 48, 49,
    57, 4, 22, 7, 9, 10, 11, 13, 14, 15, 51, 52, 50, 40,
    106, 100, 29, 27, 6, 25, 5, 17, 16, 54, 55, 56, 110,
    105, 108, 107, 44, 111, 122, 101, 109,
];

const K57: Overlay = Overlay {
    size: Size::Full,
    values: FULL_VALUES,
    edits: &[
        new_row(0, 17, 0),
        ins(0, 18, 1),
        ins(1, 0, 131),
        ins(2, 0, 132),
        ins(3, 0, 133),
        ins(4, 0, 134),
        ins(5, 0, 135),
        ins(6, 0, 136),
    ],
};

const K60: Overlay = Overlay {
    size: Size::Full,
    values: FULL_VALUES,
    edits: &[],
};

const K60_TKL: Overlay = Overlay {
    size: Size::Tkl,
    values: TKL_VALUES,
    edits: &[],
};

const K65_MINI: Overlay = Overlay {
    size: Size::Sixty,
    values: K65_MINI_VALUES,
    edits: &[swap(0, 0, 41), swap(4, 4, 0), swap(4, 8, 1)],
};

const K70: Overlay = Overlay {
    size: Size::Full,
    values: FULL_VALUES,
    edits: &[],
};

const K70_PRO: Overlay = Overlay {
    size: Size::Full,
    values: FULL_VALUES,
    edits: &[
        new_row(0, 0, 128),
        ins(0, 1, 113),
        ins(0, 2, 114),
        ins(0, 10, 138),
        ins(0, 18, 102),
        ins(1, 17, 123),
        ins(1, 18, 126),
        ins(1, 19, 124),
        ins(1, 20, 125),
        ins(6, 5, 140),
        ins(6, 7, 141),
    ],
};

const K70_TKL_CS: Overlay = Overlay {
    size: Size::Tkl,
    values: TKL_VALUES,
    edits: &[
        new_row(0, 1, 123),
        ins(0, 2, 126),
        ins(0, 3, 124),
        ins(0, 4, 125),
        ins(0, 7, 1),
        ins(0, 8, 3),
        ins(0, 11, 128),
        ins(0, 12, 113),
        ins(0, 13, 114),
        ins(0, 14, 102),
    ],
};

const K70_CORE_TKL: Overlay = Overlay {
    size: Size::Tkl,
    values: TKL_VALUES,
    edits: &[swap(0, 14, 70), unused(0, 15), swap(0, 16, 72)],
};

const K95_EDITS: &[Edit] = &[
    swap(5, 11, 112),
    ins(0, 17, 123),
    ins(0, 18, 126),
    ins(0, 19, 124),
    ins(0, 20, 125),
    new_row(0, 4, 128),
    ins(0, 5, 113),
    ins(0, 6, 114),
    ins(0, 18, 102),
    ins(1, 0, 131),
    ins(2, 0, 132),
    ins(3, 0, 133),
    ins(4, 0, 134),
    ins(5, 0, 135),
    ins(6, 0, 136),
    new_row(0, 0, 137),
    ins(0, 1, 138),
    ins(0, 2, 139),
    ins(0, 3, 140),
    ins(0, 4, 141),
    ins(0, 5, 142),
    ins(0, 6, 143),
    ins(0, 7, 144),
    ins(0, 8, 145),
    ins(0, 9, 146),
    ins(0, 10, 147),
    ins(0, 11, 148),
    ins(0, 12, 149),
    ins(0, 13, 150),
    ins(0, 14, 151),
    ins(0, 15, 152),
    ins(0, 16, 153),
    ins(0, 17, 154),
    ins(0, 18, 155),
];

const K95: Overlay = Overlay {
    size: Size::Full,
    values: FULL_VALUES,
    edits: K95_EDITS,
};

const K100_EDITS: &[Edit] = &[
    ins(0, 17, 123),
    ins(0, 18, 126),
    ins(0, 19, 124),
    ins(0, 20, 125),
    new_row(0, 1, 128),
    ins(0, 2, 137),
    ins(0, 3, 114),
    ins(0, 10, 190),
    ins(0, 11, 191),
    ins(0, 12, 192),
    ins(0, 18, 102),
    ins(1, 0, 131),
    ins(2, 0, 132),
    ins(3, 0, 133),
    ins(4, 0, 134),
    ins(5, 0, 135),
    ins(6, 0, 136),
    new_row(0, 0, 138),
    ins(0, 1, 139),
    ins(0, 2, 140),
    ins(0, 3, 141),
    ins(0, 4, 142),
    ins(0, 5, 143),
    ins(0, 6, 144),
    ins(0, 7, 145),
    ins(0, 8, 146),
    ins(0, 9, 147),
    ins(0, 10, 148),
    ins(0, 11, 149),
    ins(0, 12, 150),
    ins(0, 13, 151),
    ins(0, 14, 152),
    ins(0, 15, 153),
    ins(0, 16, 154),
    ins(0, 17, 155),
    ins(0, 18, 156),
    ins(0, 19, 157),
    ins(0, 20, 158),
    ins(0, 21, 159),
    ins(0, 0, 160),
    ins(1, 0, 161),
    ins(2, 0, 162),
    ins(3, 0, 163),
    ins(4, 0, 164),
    ins(5, 0, 165),
    ins(6, 0, 166),
    ins(7, 0, 167),
    ins(8, 0, 168),
    ins(9, 0, 169),
    ins(10, 0, 170),
    ins(0, 23, 171),
    ins(1, 23, 172),
    ins(2, 23, 173),
    ins(3, 23, 174),
    ins(4, 23, 175),
    ins(5, 23, 176),
    ins(6, 23, 177),
    ins(7, 23, 178),
    ins(8, 23, 179),
    ins(9, 23, 180),
    ins(10, 23, 181),
    new_row(0, 0, 182),
    ins(0, 1, 183),
    ins(0, 2, 184),
    ins(0, 3, 185),
    ins(0, 4, 186),
    ins(0, 5, 187),
    ins(0, 6, 188),
    ins(0, 7, 189),
];

const K100: Overlay = Overlay {
    size: Size::Full,
    values: FULL_VALUES,
    edits: K100_EDITS,
};

/// How a device's LEDs are laid out: a run of LEDs, or keys from a layout overlay.
#[derive(Clone, Copy)]
enum Shape {
    Leds(usize),
    Keys(&'static Overlay),
}

struct Spec {
    pid: u16,
    kind: DeviceKind,
    shape: Shape,
    light_ctrl_reg: u8,
    pkt_size: usize,
}

const fn spec(pid: u16, kind: DeviceKind, shape: Shape, reg: u8, pkt: usize) -> Spec {
    Spec {
        pid,
        kind,
        shape,
        light_ctrl_reg: reg,
        pkt_size: pkt,
    }
}

const KB: DeviceKind = DeviceKind::Keyboard;
const MOUSE: DeviceKind = DeviceKind::Mouse;
const PAD: DeviceKind = DeviceKind::Other;

static DEVICES: &[Spec] = &[
    spec(0x1BA4, KB, Shape::Leds(6), 0x5F, 65),
    spec(0x1B6E, KB, Shape::Keys(&K57), 0x5F, 65),
    spec(0x1BA0, KB, Shape::Keys(&K60), 0x5F, 65),
    spec(0x1B83, KB, Shape::Keys(&K60), 0x5F, 65),
    spec(0x1B8D, KB, Shape::Keys(&K60), 0x5F, 65),
    spec(0x1BAD, KB, Shape::Keys(&K60), 0x5F, 65),
    spec(0x1BC7, KB, Shape::Keys(&K60_TKL), 0x5F, 65),
    spec(0x1BED, KB, Shape::Keys(&K60_TKL), 0x5F, 65),
    spec(0x1BAF, KB, Shape::Keys(&K65_MINI), 0x6E, 1025),
    spec(0x1BFD, KB, Shape::Keys(&K70), 0x5F, 65),
    spec(0x2B01, KB, Shape::Keys(&K70_CORE_TKL), 0x5F, 65),
    spec(0x1BC4, KB, Shape::Keys(&K70_PRO), 0x5F, 65),
    spec(0x1BB3, KB, Shape::Keys(&K70_PRO), 0x5F, 1025),
    spec(0x1B73, KB, Shape::Keys(&K70_TKL_CS), 0x5F, 65),
    spec(0x1BB9, KB, Shape::Keys(&K70_TKL_CS), 0x5F, 65),
    spec(0x1B89, KB, Shape::Keys(&K95), 0x5F, 65),
    spec(0x1B7D, KB, Shape::Keys(&K100), 0x5F, 65),
    spec(0x1B7C, KB, Shape::Keys(&K100), 0x5F, 65),
    spec(0x1BC5, KB, Shape::Keys(&K100), 0x5F, 65),
    spec(0x1B4B, MOUSE, Shape::Leds(12), 0x5F, 65),
    spec(0x1B7E, MOUSE, Shape::Leds(12), 0x5F, 65),
    spec(0x1B5E, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1B4C, MOUSE, Shape::Leds(6), 0x5F, 65),
    spec(0x2B32, MOUSE, Shape::Leds(6), 0x5F, 65),
    spec(0x1B93, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1BBA, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1BAC, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1B70, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1B9E, MOUSE, Shape::Leds(3), 0x5F, 65),
    spec(0x1BB5, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1BF0, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1BF1, MOUSE, Shape::Leds(2), 0x5F, 65),
    spec(0x1BE3, MOUSE, Shape::Leds(5), 0x5F, 65),
    spec(0x1B9B, PAD, Shape::Leds(3), 0x5F, 65),
    spec(0x1BC9, PAD, Shape::Leds(3), 0x5F, 65),
];

fn blank(pkt: usize, bytes: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; pkt];
    b[1..1 + bytes.len()].copy_from_slice(bytes);
    b
}

/// GetAddress request; also what LightingControl sends with the device's register.
pub fn get(cmd: u8, address: u8, pkt: usize) -> Vec<u8> {
    blank(pkt, &[cmd, 0x02, address])
}

/// SetRenderMode
pub fn render_mode(cmd: u8, mode: u8, pkt: usize) -> Vec<u8> {
    blank(pkt, &[cmd, 0x01, 0x03, 0x00, mode])
}

/// StartTransaction
pub fn start_tx(cmd: u8, opt: u8, light_ctrl: u8, pkt: usize) -> Vec<u8> {
    blank(pkt, &[cmd, 0x0D, opt, light_ctrl])
}

/// StopTransaction
pub fn stop_tx(cmd: u8, opt: u8, pkt: usize) -> Vec<u8> {
    blank(pkt, &[cmd, 0x05, 0x01, opt])
}

/// StartTransaction's result byte: the reply overwrites the request buffer, so silence leaves 0x0D.
pub fn start_result(request: &[u8], answer: &[u8]) -> u8 {
    answer.get(2).copied().unwrap_or(request[2])
}

/// The value in a GetAddress reply, or NA when the device reports an error.
pub fn address_value(read: &[u8]) -> u32 {
    let mut r = [0u8; 7];
    let n = read.len().min(7);
    r[..n].copy_from_slice(&read[..n]);
    if r[2] > 0 {
        return NA;
    }
    u32::from_le_bytes([r[3], r[4], r[5], r[6]])
}

/// SetLEDs block packets: a header packet with the length, then continuation packets.
pub fn blocks(cmd: u8, data: &[u8], pkt: usize) -> Vec<Vec<u8>> {
    let size = data.len();
    let mut buf = blank(
        pkt,
        &[cmd, 0x06, 0x01, (size & 0xFF) as u8, (size >> 8) as u8],
    );
    let first = size.min(pkt - 8);
    buf[8..8 + first].copy_from_slice(&data[..first]);
    let mut out = vec![buf.clone()];
    let mut remaining = size - first;
    buf[2] = 0x07;
    let mut copy = pkt - 4;
    while remaining > 0 {
        let index = size - remaining;
        if remaining < copy {
            buf[4..4 + copy].fill(0);
            copy = remaining;
        }
        buf[4..4 + copy].copy_from_slice(&data[index..index + copy]);
        out.push(buf.clone());
        remaining -= copy;
    }
    out
}

/// SetLedsDirect (software controller, and hardware controller on endpoint 1): R, G, B planes.
pub fn colour_blocks(colors: &[Color]) -> Vec<u8> {
    let n = colors.len();
    let mut out = vec![0u8; n * 3];
    for (i, c) in colors.iter().enumerate() {
        out[i] = c.0;
        out[n + i] = c.1;
        out[2 * n + i] = c.2;
    }
    out
}

/// One intensity plane, the brightest channel of each colour.
pub fn mono_plane(colors: &[Color]) -> Vec<u8> {
    colors.iter().map(|c| c.0.max(c.1).max(c.2)).collect()
}

/// SetLedsDirectTriplets: the direct mode id, then RGB per LED.
pub fn triplets(colors: &[Color]) -> Vec<u8> {
    let mut out = DIRECT.to_le_bytes().to_vec();
    for c in colors {
        out.extend([c.0, c.1, c.2]);
    }
    out
}

/// The buffer map: one value per LED and the number of slots the device expects.
fn plan(shape: Shape, layout: Layout) -> (Vec<u32>, usize, Vec<[f32; 2]>, [u32; 2]) {
    match shape {
        Shape::Leds(n) => ((0..n as u32).collect(), n, line(n), [n as u32, 1]),
        Shape::Keys(overlay) => {
            let k = Keymap::build(layout, overlay);
            let values = k.values();
            let len = values.iter().max().map_or(0, |m| *m as usize) + 1;
            (values, len, k.positions(), [k.cols as u32, k.rows as u32])
        }
    }
}

/// Colours placed at each LED's value; unlisted slots stay black.
pub fn ordered(values: &[u32], len: usize, colors: &[Color]) -> Vec<Color> {
    let mut out = vec![Color::BLACK; len];
    for (&v, &c) in values.iter().zip(colors) {
        out[v as usize] = c;
    }
    out
}

/// CORSAIR_V2_KB_LAYOUT_*: ISO and JIS, everything else ANSI.
fn layout_from(value: u32) -> Layout {
    match value {
        0x02 => Layout::Iso,
        0x04 => Layout::Jis,
        _ => Layout::Ansi,
    }
}

struct V2 {
    hid: HidDevice,
    cmd: u8,
    light_ctrl: u8,
    skip_reads: bool,
    pkt: usize,
    reg: u8,
    hw: bool,
    values: Vec<u32>,
    len: usize,
    taken: bool,
    mono: bool,
}

impl V2 {
    fn send(&self, report: &[u8]) -> Result<(), String> {
        let _turn = foxden_core::hid_turn::take();
        drain(&self.hid);
        write(&self.hid, report)?;
        if !self.skip_reads {
            read(&self.hid, TIMEOUT)?;
        }
        Ok(())
    }

    fn set_leds(&self, data: &[u8]) -> Result<(), String> {
        self.send(&start_tx(self.cmd, 1, self.light_ctrl, self.pkt))?;
        let packets = blocks(self.cmd, data, self.pkt);
        let turn = foxden_core::hid_turn::take();
        drain(&self.hid);
        write(&self.hid, &packets[0])?;
        if !self.skip_reads && read(&self.hid, TIMEOUT_SHORT)?.is_empty() {
            sleep(WRITE_PACE);
        }
        for p in &packets[1..] {
            write(&self.hid, p)?;
            sleep(WRITE_PACE);
        }
        drop(turn);
        self.send(&stop_tx(self.cmd, 0, self.pkt))
    }
}

impl Driver for V2 {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            self.send(&render_mode(self.cmd, MODE_SW, self.pkt))?;
            self.send(&get(self.cmd, self.reg, self.pkt))?;
            self.taken = true;
        }
        let buffer = ordered(&self.values, self.len, colors);
        let data = if self.mono {
            mono_plane(&buffer)
        } else if self.hw && self.light_ctrl == LIGHT_CTRL2 {
            triplets(&buffer)
        } else {
            colour_blocks(&buffer)
        };
        self.set_leds(&data)
    }
}

impl Drop for V2 {
    fn drop(&mut self) {
        if self.taken {
            let _ = self.send(&render_mode(self.cmd, MODE_HW, self.pkt));
        }
    }
}

/// Drops input reports left over from other readers of this device, such as battery queries.
fn drain(hid: &HidDevice) {
    let mut buf = [0u8; 1025];
    for _ in 0..64 {
        if !matches!(hid.read_timeout(&mut buf, 0), Ok(n) if n > 0) {
            break;
        }
    }
}

fn get_address(hid: &HidDevice, cmd: u8, address: u8, pkt: usize) -> Result<u32, String> {
    let _turn = foxden_core::hid_turn::take();
    drain(hid);
    write(hid, &get(cmd, address, pkt))?;
    Ok(address_value(&read(hid, TIMEOUT)?))
}

/// GetAddressRetry: a receiver answers 0 or an error until its device wakes.
fn get_address_retry(hid: &HidDevice, cmd: u8, address: u8, pkt: usize) -> Result<u32, String> {
    let mut value = NA;
    for attempt in 0..8 {
        value = get_address(hid, cmd, address, pkt)?;
        if value != NA && value != 0 {
            return Ok(value);
        }
        if attempt < 7 {
            sleep(Duration::from_millis(120));
        }
    }
    Ok(value)
}

fn open_sw(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, det, false)
}

fn open_hw(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, det, true)
}

fn open(api: &HidApi, info: &Info, det: &Detector, hw: bool) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let mut cmd = WIRED;
    let mut light_ctrl = LIGHT_CTRL2;
    let mut skip_reads = false;
    let mut pid = info.pid as u32;
    if RECEIVERS.contains(&info.pid) {
        cmd = WIRELESS;
        pid = get_address_retry(&hid, cmd, 0x12, 1025)?;
    } else if info.pid == K57_WIRED {
        cmd = 0x80;
        light_ctrl = LIGHT_CTRL1;
        skip_reads = true;
    }
    let spec = DEVICES
        .iter()
        .find(|d| d.pid as u32 == pid)
        .ok_or_else(|| format!("capabilities unknown for {pid:04x}"))?;
    let pkt = spec.pkt_size;
    let mut driver = V2 {
        hid,
        cmd,
        light_ctrl,
        skip_reads,
        pkt,
        reg: spec.light_ctrl_reg,
        hw,
        values: Vec::new(),
        len: 0,
        taken: false,
        mono: MONO.contains(&spec.pid),
    };
    if driver.mono {
        driver.light_ctrl = LIGHT_MONO;
    } else if light_ctrl == LIGHT_CTRL2 && !PINNED.contains(&spec.pid) {
        let request = start_tx(cmd, 0, driver.light_ctrl, pkt);
        let turn = foxden_core::hid_turn::take();
        drain(&driver.hid);
        write(&driver.hid, &request)?;
        let answer = if skip_reads {
            Vec::new()
        } else {
            read(&driver.hid, TIMEOUT)?
        };
        drop(turn);
        if start_result(&request, &answer) > 0 {
            driver.light_ctrl = LIGHT_CTRL1;
            driver.send(&start_tx(cmd, 0, driver.light_ctrl, pkt))?;
        }
        driver.send(&stop_tx(cmd, 0, pkt))?;
    }
    let layout = match spec.shape {
        Shape::Keys(_) => layout_from(get_address(&driver.hid, cmd, 0x41, pkt)?),
        Shape::Leds(_) => Layout::Ansi,
    };
    let (values, len, leds, grid) = plan(spec.shape, layout);
    driver.values = values;
    driver.len = len;
    let dev = device(info.device_id("corsairv2"), det.name, spec.kind, leds, grid);
    Ok(Lights::boxed(
        "corsair v2",
        dev,
        Some(UPDATE_PERIOD),
        driver,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CorsairPeripheralV2Controller::SetRenderMode
    #[test]
    fn render_mode_bytes() {
        let r = render_mode(WIRED, MODE_SW, 65);
        assert_eq!(r.len(), 65);
        assert_eq!(&r[..6], &[0x00, 0x08, 0x01, 0x03, 0x00, 0x02]);
        assert!(r[6..].iter().all(|&b| b == 0));
        assert_eq!(
            &render_mode(WIRELESS, MODE_HW, 1025)[..6],
            &[0, 0x09, 1, 3, 0, 1]
        );
    }

    // CorsairPeripheralV2Controller::LightingControl and GetAddress
    #[test]
    fn get_bytes() {
        let r = get(WIRED, 0x5F, 65);
        assert_eq!(&r[..6], &[0x00, 0x08, 0x02, 0x5F, 0x00, 0x00]);
        assert_eq!(get(WIRELESS, 0x12, 1025).len(), 1025);
    }

    // CorsairPeripheralV2Controller::StartTransaction and StopTransaction
    #[test]
    fn transactions() {
        assert_eq!(
            &start_tx(WIRED, 1, LIGHT_CTRL2, 65)[..5],
            &[0, 0x08, 0x0D, 0x01, 0x22]
        );
        assert_eq!(&stop_tx(WIRED, 0, 65)[..5], &[0, 0x08, 0x05, 0x01, 0x00]);
    }

    // CorsairPeripheralV2Controller constructor: endpoint probe through StartTransaction's result.
    #[test]
    fn endpoint_probe_result() {
        let req = start_tx(WIRED, 0, LIGHT_CTRL2, 65);
        assert_eq!(start_result(&req, &[0x00, 0x08, 0x00]), 0);
        assert_eq!(start_result(&req, &[0x00, 0x08, 0x05]), 5);
        assert_eq!(start_result(&req, &[]), 0x0D);
    }

    // CorsairPeripheralV2Controller::GetAddress (reply)
    #[test]
    fn address_reply() {
        assert_eq!(
            address_value(&[0x00, 0x09, 0x00, 0x4B, 0x1B, 0x00, 0x00]),
            0x1B4B
        );
        assert_eq!(
            address_value(&[0x00, 0x09, 0x05, 0x4B, 0x1B, 0x00, 0x00]),
            NA
        );
        assert_eq!(address_value(&[]), 0);
    }

    // CorsairPeripheralV2Controller::SetLEDs (single packet)
    #[test]
    fn one_block() {
        let data: Vec<u8> = (1..=36).collect();
        let p = blocks(WIRED, &data, 65);
        assert_eq!(p.len(), 1);
        assert_eq!(&p[0][..8], &[0, 0x08, 0x06, 0x01, 36, 0, 0, 0]);
        assert_eq!(&p[0][8..44], &data[..]);
        assert!(p[0][44..].iter().all(|&b| b == 0));
    }

    // CorsairPeripheralV2Controller::SetLEDs (continuation packets, last one zero padded)
    #[test]
    fn many_blocks() {
        let data: Vec<u8> = (0..200).map(|i| i as u8).collect();
        let p = blocks(WIRED, &data, 65);
        assert_eq!(p.len(), 4);
        assert_eq!(&p[0][..8], &[0, 0x08, 0x06, 0x01, 200, 0, 0, 0]);
        assert_eq!(&p[0][8..], &data[..57]);
        assert_eq!(&p[1][..4], &[0, 0x08, 0x07, 0x01]);
        assert_eq!(&p[1][4..], &data[57..118]);
        assert_eq!(&p[2][4..], &data[118..179]);
        assert_eq!(&p[3][4..25], &data[179..200]);
        assert!(p[3][25..].iter().all(|&b| b == 0));
        let big = blocks(WIRED, &vec![7u8; 1500], 1025);
        assert_eq!(big.len(), 2);
        assert_eq!(&big[0][..6], &[0, 0x08, 0x06, 0x01, 0xDC, 0x05]);
        assert_eq!(big[1][4 + 1500 - 1017 - 1], 7);
        assert_eq!(big[1][4 + 1500 - 1017], 0);
    }

    // CorsairPeripheralV2SWController::SetLedsDirect
    #[test]
    fn mono_plane_bytes() {
        let colors = [Color(0x10, 0x20, 0x05), Color(0xFF, 0, 0), Color::BLACK];
        assert_eq!(mono_plane(&colors), vec![0x20, 0xFF, 0x00]);
        let blocks = blocks(WIRED, &mono_plane(&colors), 65);
        assert_eq!(blocks.len(), 1);
        assert!(DEVICES.iter().any(|d| d.pid == MONO[0]));
        let open = start_tx(WIRED, 1, LIGHT_MONO, 65);
        assert_eq!(open[..5], [0x00, 0x08, 0x0D, 0x01, 0x10]);
    }

    #[test]
    fn colour_block_order() {
        let d = colour_blocks(&[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(d, vec![1, 4, 2, 5, 3, 6]);
    }

    // CorsairPeripheralV2HWController::SetLedsDirectTriplets
    #[test]
    fn triplet_order() {
        let d = triplets(&[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(d, vec![0x12, 0x00, 1, 2, 3, 4, 5, 6]);
    }

    // RGBController_CorsairV2SW::SetupZones buffer_map: colours land at each key's value.
    #[test]
    fn buffer_map() {
        let b = ordered(&[2, 0], 4, &[Color(1, 1, 1), Color(2, 2, 2)]);
        assert_eq!(
            b,
            vec![Color(2, 2, 2), Color::BLACK, Color(1, 1, 1), Color::BLACK]
        );
    }

    // RGBController_CorsairV2HW::SetupZones with KeyboardLayoutManager: key counts and buffer sizes.
    #[test]
    fn keyboard_plans() {
        let (v, len, leds, _) = plan(Shape::Keys(&K70), Layout::Ansi);
        assert_eq!((v.len(), len, leds.len()), (104, 123, 104));
        assert!(!v.contains(&50) && !v.contains(&100));
        let (v, len, _, _) = plan(Shape::Keys(&K70), Layout::Iso);
        assert_eq!((v.len(), len), (105, 123));
        assert!(v.contains(&50) && v.contains(&100) && !v.contains(&49));
        let (v, len, _, _) = plan(Shape::Keys(&K65_MINI), Layout::Ansi);
        assert_eq!((v.len(), len, v[0]), (63, 123, 41));
        let (v, len, _, _) = plan(Shape::Keys(&K100), Layout::Ansi);
        assert_eq!((v.len(), len), (173, 193));
        let (v, _, _, _) = plan(Shape::Keys(&K70_TKL_CS), Layout::Ansi);
        assert_eq!(
            &v[..11],
            &[123, 126, 124, 125, 1, 3, 128, 113, 114, 102, 41]
        );
        let (v, _, _, _) = plan(Shape::Keys(&K70_CORE_TKL), Layout::Ansi);
        assert_eq!(v.len(), 86);
        assert!(!v.contains(&71));
        let (v, len, _, _) = plan(Shape::Leds(12), Layout::Ansi);
        assert_eq!((v.len(), len), (12, 12));
    }
}
