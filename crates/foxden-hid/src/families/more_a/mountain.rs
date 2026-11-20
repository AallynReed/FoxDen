//! Ported from OpenRGB Controllers/MountainKeyboardController (GPL-2.0-or-later).

use std::time::Duration;

use super::common::*;
use super::klm::{Key, Klm, Op::*, SIXTY, k};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "mountain";
const VID: u16 = 0x3282;

const TKL_MAP: [&[u32]; 6] = [
    &[
        0, NA, 8, 14, 19, 24, NA, 30, 36, 40, 45, NA, 53, 59, 65, 70, 74, 78, 83,
    ],
    &[
        1, 6, 9, 15, 20, 25, 29, 31, 37, 41, 46, NA, 54, 60, 66, NA, 75, 79, 84,
    ],
    &[
        2, NA, 10, 16, 21, 26, NA, 32, 38, 42, 47, 50, 55, 61, 67, 71, 76, 80, 85,
    ],
    &[
        3, NA, 11, 17, 22, 27, NA, 33, 39, 43, 48, 51, 56, 62, NA, 72, NA, NA, NA,
    ],
    &[
        4, NA, 12, 18, 23, 28, NA, 34, NA, 44, 49, 52, 57, 63, 68, NA, NA, 81, NA,
    ],
    &[
        5, 7, 13, NA, NA, NA, NA, 35, NA, NA, NA, NA, 58, 64, 69, 73, 77, 82, 86,
    ],
];

const NUM_MAP: [&[u32]; 6] = [
    &[NA, NA, NA, NA],
    &[0, 5, 9, 14],
    &[1, 6, 10, 15],
    &[2, 7, 11, NA],
    &[3, 8, 12, 16],
    &[4, NA, 13, NA],
];

const EDGE_TKL_MAP: [&[u32]; 6] = [
    &[NA, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, NA],
    &[0, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 28],
    &[1, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 29],
    &[2, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 30],
    &[3, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 31],
    &[NA, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, NA],
];

const EDGE_NUM_MAP: [&[u32]; 6] = [
    &[NA, 4, 6, 8, NA],
    &[0, NA, NA, NA, 10],
    &[1, NA, NA, NA, 11],
    &[2, NA, NA, NA, 12],
    &[3, NA, NA, NA, 13],
    &[NA, 5, 7, 9, NA],
];

/// `matrix_tkl_map_lut`: each keyboard LED's slot in the colour buffer.
const TKL_LUT: [u8; 87] = [
    0, 1, 2, 3, 4, 5, 10, 14, 9, 19, 11, 12, 22, 23, 18, 28, 20, 21, 31, 27, 37, 29, 30, 40, 36,
    46, 38, 39, 49, 55, 45, 64, 47, 48, 58, 41, 54, 73, 56, 57, 63, 82, 65, 66, 67, 72, 91, 74, 75,
    76, 83, 84, 85, 81, 100, 92, 93, 94, 68, 90, 109, 101, 102, 103, 77, 99, 87, 110, 121, 86, 108,
    119, 120, 95, 117, 96, 88, 104, 114, 105, 97, 124, 113, 123, 115, 106, 122,
];

const NUM_LUT: [u8; 17] = [
    6, 61, 51, 34, 78, 24, 69, 52, 42, 15, 70, 60, 43, 79, 16, 7, 33,
];

const EDGE_TKL_LUT: [u8; 32] = [
    16, 17, 18, 19, 13, 20, 14, 21, 15, 22, 7, 23, 6, 24, 5, 25, 4, 26, 3, 27, 2, 28, 1, 29, 0, 30,
    45, 12, 9, 8, 10, 11,
];

const EDGE_NUM_LUT: [u8; 14] = [31, 32, 33, 34, 44, 35, 43, 36, 42, 37, 41, 40, 39, 38];

const PAYLOAD: usize = 57;
const MODE_CUSTOM: u8 = 0x06;

/// RGBController_MountainKeyboard::DeviceUpdate for Direct: keys and edges into their buffers.
pub fn buffers(colors: &[Color]) -> ([u8; 378], [u8; 138]) {
    let mut keys = [0u8; 378];
    let mut edges = [0u8; 138];
    let zones: [(&[u8], bool); 4] = [
        (&TKL_LUT, false),
        (&NUM_LUT, false),
        (&EDGE_TKL_LUT, true),
        (&EDGE_NUM_LUT, true),
    ];
    let mut led = 0;
    for (lut, edge) in zones {
        for &slot in lut {
            let buf: &mut [u8] = if edge { &mut edges } else { &mut keys };
            let s = slot as usize * 3;
            buf[s..s + 3].copy_from_slice(&rgb(colors[led]));
            led += 1;
        }
    }
    (keys, edges)
}

/// SendColorPacketCmd / SendColorEdgePacketCmd: an 8-byte header and up to 57 colour bytes.
fn packet(edge: bool, pkt: u8, brightness: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = 0x14;
    if edge {
        b[2] = 0x2D;
        b[3] = 0x0A;
        b[6] = 0xFF;
    } else {
        b[2] = 0x2C;
        b[4] = 0x01;
        b[6] = brightness;
    }
    b[5] = pkt;
    b[8..8 + data.len()].copy_from_slice(data);
    b
}

struct Everest {
    prev_keys: Option<[u8; 378]>,
    prev_edges: Option<[u8; 138]>,
}

/// SendDirectColorCmd / SendDirectColorEdgeCmd in quick mode: only the chunks that changed.
fn quick(io: &mut dyn Io, edge: bool, data: &[u8], prev: Option<&[u8]>) -> Result<(), String> {
    for (pkt, chunk) in data.chunks(PAYLOAD).enumerate() {
        let at = pkt * PAYLOAD;
        if prev.is_none_or(|p| p[at..at + chunk.len()] != *chunk) {
            io.write(&packet(edge, pkt as u8, 0, chunk))?;
            io.sleep(5);
        }
    }
    Ok(())
}

impl Proto for Everest {
    /// DeviceUpdateMode: SelectMode(Direct) the first time.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&pad(&[0, 0x14, 0x00, 0, 0, 0x01, MODE_CUSTOM], 65))?;
        io.sleep(200);
        Ok(())
    }

    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let (keys, edges) = buffers(colors);
        quick(io, false, &keys, self.prev_keys.as_ref().map(|p| &p[..]))?;
        self.prev_keys = Some(keys);
        quick(io, true, &edges, self.prev_edges.as_ref().map(|p| &p[..]))?;
        self.prev_edges = Some(edges);
        Ok(())
    }
}

/// Keys, numpad and both edge rings on one 27 x 8 canvas.
fn everest_leds() -> Vec<[f32; 2]> {
    let (w, h) = (27.0, 8.0);
    let mut leds = Vec::new();
    let mut add = |map: &[&[u32]], x0: f32, y0: f32, dx: f32, dy: f32| {
        let mut at = vec![
            [0.0f32; 2];
            map.iter()
                .flat_map(|r| r.iter())
                .filter(|&&v| v != NA)
                .count()
        ];
        for (r, row) in map.iter().enumerate() {
            for (c, &v) in row.iter().enumerate() {
                if v != NA {
                    at[v as usize] = [(x0 + c as f32 * dx) / w, (y0 + r as f32 * dy) / h];
                }
            }
        }
        leds.extend(at);
    };
    add(&TKL_MAP, 1.5, 1.5, 1.0, 1.0);
    add(&NUM_MAP, 22.5, 1.5, 1.0, 1.0);
    add(&EDGE_TKL_MAP, 0.5, 0.5, 20.0 / 13.0, 7.0 / 5.0);
    add(&EDGE_NUM_MAP, 21.5, 0.5, 5.0 / 4.0, 7.0 / 5.0);
    leds
}

fn open_everest(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        FAMILY,
        d,
        DeviceKind::Keyboard,
        [27, 8],
        everest_leds(),
    );
    let proto = Everest {
        prev_keys: None,
        prev_edges: None,
    };
    Ok(boxed(opened(api, info)?, FAMILY, dev, proto))
}

const VALUES_60: [u32; 64] = [
    0, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
    52, 53, 54, 55, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 76, 84, 85, 86, 87, 88, 89, 90,
    91, 92, 93, 94, 97, 99, 56, 105, 106, 107, 110, 113, 115, 119, 120, 121,
];

/// `mountain60_keyboard_overlay_no_numpad`.
const OVERLAY_60: [Key; 105] = [
    k(4, 13, 120, "Down Arrow", SwapOnly),
    k(4, 12, 119, "Left Arrow", SwapOnly),
    k(4, 14, 121, "Right Arrow", SwapOnly),
    k(4, 14, 121, "Right Arrow", InsertShiftRight),
    k(3, 13, 99, "Up Arrow", SwapOnly),
    k(3, 11, 94, "/", SwapOnly),
    k(3, 12, 97, "Right Shift", SwapOnly),
    k(3, 2, 85, "Z", SwapOnly),
    k(3, 3, 86, "X", SwapOnly),
    k(3, 4, 87, "C", SwapOnly),
    k(3, 5, 88, "B", SwapOnly),
    k(3, 6, 89, "V", SwapOnly),
    k(3, 7, 90, "N", SwapOnly),
    k(3, 8, 91, "M", SwapOnly),
    k(3, 9, 92, ",", SwapOnly),
    k(3, 10, 93, ".", SwapOnly),
    k(3, 14, 56, "Delete", SwapOnly),
    k(2, 13, 76, "Enter", SwapOnly),
    k(3, 0, 84, "Left Shift", SwapOnly),
    k(0, 0, 126, "Edge 1", InsertRow),
    k(0, 1, 127, "Edge 2", InsertShiftRight),
    k(0, 2, 128, "Edge 3", InsertShiftRight),
    k(0, 3, 129, "Edge 4", InsertShiftRight),
    k(0, 4, 130, "Edge 5", InsertShiftRight),
    k(0, 5, 131, "Edge 6", InsertShiftRight),
    k(0, 6, 132, "Edge 7", InsertShiftRight),
    k(0, 7, 133, "Edge 8", InsertShiftRight),
    k(0, 8, 134, "Edge 9", InsertShiftRight),
    k(0, 9, 135, "Edge 10", InsertShiftRight),
    k(0, 10, 136, "Edge 11", InsertShiftRight),
    k(0, 11, 137, "Edge 12", InsertShiftRight),
    k(0, 12, 138, "Edge 13", InsertShiftRight),
    k(0, 13, 139, "Edge 14", InsertShiftRight),
    k(0, 14, 140, "Edge 15", InsertShiftRight),
    k(0, 15, 141, "Edge 16", InsertShiftRight),
    k(0, 0, 169, "Edge 44", InsertShiftRight),
    k(1, 0, 168, "Edge 43", InsertShiftRight),
    k(2, 0, 167, "Edge 42", InsertShiftRight),
    k(3, 0, 166, "Edge 41", InsertShiftRight),
    k(4, 0, 165, "Edge 40", InsertShiftRight),
    k(5, 0, 164, "Edge 39", InsertShiftRight),
    k(6, 0, 1, "", InsertRow),
    k(6, 0, 163, "Edge 38", InsertShiftRight),
    k(6, 1, 162, "Edge 37", InsertShiftRight),
    k(6, 2, 161, "Edge 36", InsertShiftRight),
    k(6, 3, 160, "Edge 35", InsertShiftRight),
    k(6, 4, 159, "Edge 34", InsertShiftRight),
    k(6, 5, 158, "Edge 33", InsertShiftRight),
    k(6, 6, 157, "Edge 32", InsertShiftRight),
    k(6, 7, 156, "Edge 31", InsertShiftRight),
    k(6, 8, 155, "Edge 30", InsertShiftRight),
    k(6, 9, 154, "Edge 29", InsertShiftRight),
    k(6, 10, 153, "Edge 28", InsertShiftRight),
    k(6, 11, 152, "Edge 27", InsertShiftRight),
    k(6, 12, 151, "Edge 26", InsertShiftRight),
    k(6, 13, 150, "Edge 25", InsertShiftRight),
    k(6, 14, 149, "Edge 24", InsertShiftRight),
    k(6, 15, 148, "Edge 23", InsertShiftRight),
    k(1, 16, 142, "Edge 17", InsertShiftRight),
    k(2, 16, 143, "Edge 18", InsertShiftRight),
    k(3, 16, 144, "Edge 19", InsertShiftRight),
    k(4, 16, 145, "Edge 20", InsertShiftRight),
    k(5, 16, 146, "Edge 21", InsertShiftRight),
    k(6, 16, 147, "Edge 22", InsertShiftRight),
    k(6, 16, 147, "Edge 22", SwapOnly),
    k(0, 17, 191, "Numpad Edge 22", InsertShiftRight),
    k(1, 17, 190, "Numpad Edge 21", InsertShiftRight),
    k(2, 17, 189, "Numpad Edge 20", InsertShiftRight),
    k(3, 17, 188, "Numpad Edge 19", InsertShiftRight),
    k(4, 17, 187, "Numpad Edge 18", InsertShiftRight),
    k(5, 17, 186, "Numpad Edge 17", InsertShiftRight),
    k(0, 18, 170, "Numpad Edge 1", InsertShiftRight),
    k(0, 19, 171, "Numpad Edge 2", InsertShiftRight),
    k(0, 20, 172, "Numpad Edge 3", InsertShiftRight),
    k(0, 21, 173, "Numpad Edge 4", InsertShiftRight),
    k(0, 22, 174, "Numpad Edge 5", InsertShiftRight),
    k(6, 17, 185, "Numpad Edge 12", InsertShiftRight),
    k(6, 18, 184, "Numpad Edge 13", InsertShiftRight),
    k(6, 19, 183, "Numpad Edge 14", InsertShiftRight),
    k(6, 20, 182, "Numpad Edge 15", InsertShiftRight),
    k(6, 21, 181, "Numpad Edge 16", InsertShiftRight),
    k(1, 22, 175, "Numpad Edge 6", InsertShiftRight),
    k(2, 22, 176, "Numpad Edge 7", InsertShiftRight),
    k(3, 22, 177, "Numpad Edge 8", InsertShiftRight),
    k(4, 22, 178, "Numpad Edge 9", InsertShiftRight),
    k(5, 22, 179, "Numpad Edge 10", InsertShiftRight),
    k(6, 22, 180, "Numpad Edge 11", InsertShiftRight),
    k(6, 22, 180, "Numpad Edge 11", SwapOnly),
    k(1, 18, 38, "Num Lock", SwapOnly),
    k(1, 19, 39, "Number Pad /", SwapOnly),
    k(1, 20, 40, "Number Pad *", SwapOnly),
    k(1, 21, 41, "Number Pad -", SwapOnly),
    k(2, 18, 59, "Number Pad 7", SwapOnly),
    k(2, 19, 60, "Number Pad 8", SwapOnly),
    k(2, 20, 61, "Number Pad 9", SwapOnly),
    k(2, 21, 62, "Number Pad +", SwapOnly),
    k(3, 18, 80, "Number Pad 4", SwapOnly),
    k(3, 19, 81, "Number Pad 5", SwapOnly),
    k(3, 20, 82, "Number Pad 6", SwapOnly),
    k(4, 18, 101, "Number Pad 1", SwapOnly),
    k(4, 19, 102, "Number Pad 2", SwapOnly),
    k(4, 20, 103, "Number Pad 3", SwapOnly),
    k(5, 18, 122, "Number Pad 0", SwapOnly),
    k(5, 20, 124, "Number Pad .", SwapOnly),
    k(5, 21, 125, "Number Pad Enter", SwapOnly),
];

/// RGBController_Mountain60Keyboard::SetupZones.
pub fn layout_60() -> Klm {
    let mut kb = Klm::ansi(SIXTY, &VALUES_60);
    kb.change_keys(&OVERLAY_60);
    kb
}

const MAGIC: [u8; 3] = [0x46, 0x23, 0xEA];
const MODE_CUSTOM_60: u8 = 0x07;
const BRIGHTNESS_60: u8 = 4;
const CHUNK_60: usize = 56;

/// A Mountain60KeyboardController feature report: command, the magic bytes, then `rest` from byte 5.
fn cmd60(cmd: u8, rest: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = cmd;
    b[2..5].copy_from_slice(&MAGIC);
    b[5..5 + rest.len()].copy_from_slice(rest);
    b
}

/// Sends a feature report and reads the 65-byte answer, as most Mountain60 calls do.
fn ask60(io: &mut dyn Io, report: &[u8]) -> Result<(), String> {
    io.feature(report)?;
    let mut read = [0u8; 65];
    io.get_feature(&mut read)?;
    Ok(())
}

struct Everest60 {
    values: Vec<u32>,
}

impl Proto for Everest60 {
    /// DeviceUpdateMode for Direct: SelectMode, then SendModeDetails (which also saves).
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        ask60(io, &cmd60(0x16, &[0x01, 0, 0, 0, MODE_CUSTOM_60]))?;
        io.feature(&cmd60(
            0x17,
            &[MODE_CUSTOM_60, 0, 0, BRIGHTNESS_60 * 25, 0x00, 0x04],
        ))?;
        io.sleep(10);
        ask60(io, &cmd60(0x1A, &[MODE_CUSTOM_60]))
    }

    /// DeviceUpdateLEDs: SendDirect with [value, r, g, b] per LED.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let data: Vec<u8> = self
            .values
            .iter()
            .zip(colors)
            .flat_map(|(&v, c)| [v as u8, c.0, c.1, c.2])
            .collect();
        ask60(io, &cmd60(0x34, &[BRIGHTNESS_60 * 25, 0xC0]))?;
        for chunk in data.chunks(CHUNK_60) {
            let flag = if chunk.len() == CHUNK_60 { 0x0E } else { 0x0A };
            let mut b = cmd60(0x35, &[flag]);
            b[9..9 + chunk.len()].copy_from_slice(chunk);
            b[9 + chunk.len()..].fill(0xFF);
            ask60(io, &b)?;
        }
        ask60(io, &cmd60(0x36, &[]))
    }

    /// UpdateMountain thread: UpdateData every 500 ms.
    fn keepalive(&self) -> Option<Keepalive> {
        Some(Keepalive {
            idle: Duration::ZERO,
            every: Duration::from_millis(500),
        })
    }

    fn refresh(&mut self, io: &mut dyn Io) -> Result<(), String> {
        ask60(io, &cmd60(0x08, &[]))
    }
}

/// Mountain60KeyboardController constructor.
fn reset60(io: &mut dyn Io) -> Result<(), String> {
    ask60(io, &cmd60(0x03, &[]))
}

fn open_everest60(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    reset60(io.as_mut())?;
    let kb = layout_60();
    let (leds, grid) = kb.positions();
    let dev = placed(info, FAMILY, d, DeviceKind::Keyboard, grid, leds);
    let proto = Everest60 {
        values: kb.values(),
    };
    Ok(boxed(io, FAMILY, dev, proto))
}

pub static DETECTORS: [Detector; 3] = [
    det(
        "Mountain Everest",
        VID,
        0x0001,
        3,
        0xFF00,
        0x01,
        open_everest,
    ),
    det(
        "Mountain Everest 60",
        VID,
        0x0005,
        2,
        0xFFFF,
        0x01,
        open_everest60,
    ),
    det(
        "Mountain Everest 60",
        VID,
        0x0006,
        2,
        0xFFFF,
        0x01,
        open_everest60,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    const N: usize = 87 + 17 + 32 + 14;

    #[test]
    fn maps_and_luts_agree() {
        for (map, lut) in [
            (&TKL_MAP[..], TKL_LUT.len()),
            (&NUM_MAP[..], NUM_LUT.len()),
            (&EDGE_TKL_MAP[..], EDGE_TKL_LUT.len()),
            (&EDGE_NUM_MAP[..], EDGE_NUM_LUT.len()),
        ] {
            let (ids, _, _) = mapped(map);
            assert_eq!(ids, (0..lut as u32).collect::<Vec<_>>());
        }
        assert_eq!(everest_leds().len(), N);
    }

    fn everest() -> Everest {
        Everest {
            prev_keys: None,
            prev_edges: None,
        }
    }

    fn key_packet(pkt: u8, data: &[u8]) -> Op {
        let mut b = pad(&[0, 0x14, 0x2C, 0, 0x01, pkt, 0, 0], 65);
        b[8..8 + data.len()].copy_from_slice(data);
        w(b)
    }

    fn edge_packet(pkt: u8, data: &[u8]) -> Op {
        let mut b = pad(&[0, 0x14, 0x2D, 0x0A, 0, pkt, 0xFF, 0], 65);
        b[8..8 + data.len()].copy_from_slice(data);
        w(b)
    }

    // DeviceUpdateMode (SelectMode 6) then DeviceUpdate in quick mode: every chunk on the first call
    #[test]
    fn first_frame_sends_all_chunks() {
        let mut colors = vec![Color::BLACK; N];
        colors[0] = Color(1, 2, 3);
        colors[6] = Color(4, 5, 6);
        colors[87] = Color(7, 8, 9);
        colors[104] = Color(10, 11, 12);
        let [first, _, _] = pushes(everest(), N, vec![], &colors, &colors);
        assert_eq!(first[0], w(pad(&[0, 0x14, 0, 0, 0, 1, 6], 65)));
        assert_eq!(first[1], Op::Sleep(200));
        assert_eq!(first.len(), 2 + 2 * 10);
        let mut keys = [0u8; 378];
        keys[0..3].copy_from_slice(&[1, 2, 3]);
        keys[30..33].copy_from_slice(&[4, 5, 6]);
        keys[18..21].copy_from_slice(&[7, 8, 9]);
        let mut edges = [0u8; 138];
        edges[48..51].copy_from_slice(&[10, 11, 12]);
        for p in 0..7 {
            let end = ((p + 1) * 57).min(378);
            assert_eq!(first[2 + p * 2], key_packet(p as u8, &keys[p * 57..end]));
            assert_eq!(first[3 + p * 2], Op::Sleep(5));
        }
        for p in 0..3 {
            let end = ((p + 1) * 57).min(138);
            assert_eq!(first[16 + p * 2], edge_packet(p as u8, &edges[p * 57..end]));
        }
    }

    // SendDirectColorCmd(quick_mode): later frames send only changed chunks
    #[test]
    fn later_frames_send_changed_chunks() {
        let a = vec![Color::BLACK; N];
        let mut b = a.clone();
        b[86] = Color(1, 1, 1);
        b[N - 1] = Color(2, 2, 2);
        let [_, again, next] = pushes(everest(), N, vec![], &a, &b);
        assert!(again.is_empty());
        let mut keys = [0u8; 378];
        keys[366..369].copy_from_slice(&[1, 1, 1]);
        let mut edges = [0u8; 138];
        edges[114..117].copy_from_slice(&[2, 2, 2]);
        assert_eq!(
            next,
            vec![
                key_packet(6, &keys[342..378]),
                Op::Sleep(5),
                edge_packet(2, &edges[114..138]),
                Op::Sleep(5),
            ]
        );
    }

    // RGBController_Mountain60Keyboard::SetupZones through KeyboardLayoutManager
    #[test]
    fn everest60_layout() {
        let kb = layout_60();
        let v = kb.values();
        assert_eq!(v.len(), 147);
        assert_eq!(&v[..4], &[169, 126, 127, 128]);
        assert_eq!(&v[23..26], &[168, 0, 22]);
        let find = |n: &str| kb.keys.iter().find(|k| k.name == n).unwrap();
        assert_eq!(find("Escape").value, 0);
        assert_eq!(find("Enter").value, 76);
        assert_eq!(find("Left Shift").value, 84);
        assert_eq!(find("Space").value, 110);
        assert_eq!(find("Number Pad Enter").value, 125);
        let mut sorted = v.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), v.len());
    }

    // Mountain60KeyboardController::UpdateData, sent by the keepalive thread
    #[test]
    fn everest60_keepalive() {
        let mut rec = Rec::default();
        let mut proto = Everest60 { values: vec![] };
        assert_eq!(proto.keepalive().unwrap().every, Duration::from_millis(500));
        proto.refresh(&mut rec).unwrap();
        assert_eq!(
            rec.take(),
            vec![f(pad(&[0, 0x08, 0x46, 0x23, 0xEA], 65)), Op::GetF(0, 65)]
        );
    }

    // Mountain60KeyboardController constructor
    #[test]
    fn everest60_detection_reset() {
        let mut rec = Rec::default();
        reset60(&mut rec).unwrap();
        assert_eq!(
            rec.take(),
            vec![f(pad(&[0, 0x03, 0x46, 0x23, 0xEA], 65)), Op::GetF(0, 65)]
        );
    }

    // DeviceUpdateMode (SelectMode, SendModeDetails, SaveData) then SendDirect
    #[test]
    fn everest60_setup_and_frame() {
        let proto = Everest60 {
            values: vec![5, 200],
        };
        let [first, _, _] = pushes(proto, 2, vec![], &[Color(1, 2, 3), Color(4, 5, 6)], &[]);
        let magic = |cmd: u8, rest: &[u8]| {
            let mut h = vec![0, cmd, 0x46, 0x23, 0xEA];
            h.extend_from_slice(rest);
            pad(&h, 65)
        };
        let mut chunk = magic(0x35, &[0x0A, 0, 0, 0, 5, 1, 2, 3, 200, 4, 5, 6]);
        chunk[17..].fill(0xFF);
        assert_eq!(
            first,
            [
                f(magic(0x16, &[1, 0, 0, 0, 7])),
                Op::GetF(0, 65),
                f(magic(0x17, &[7, 0, 0, 100, 0, 4])),
                Op::Sleep(10),
                f(magic(0x1A, &[7])),
                Op::GetF(0, 65),
                f(magic(0x34, &[100, 0xC0])),
                Op::GetF(0, 65),
                f(chunk),
                Op::GetF(0, 65),
                f(magic(0x36, &[])),
                Op::GetF(0, 65),
            ]
        );
    }

    // SendDirect splits 56-byte chunks: 0x0E for full ones, 0x0A for the rest
    #[test]
    fn everest60_chunk_flags() {
        let kb = layout_60();
        let n = kb.values().len();
        let proto = Everest60 {
            values: kb.values(),
        };
        let colors = vec![Color(9, 9, 9); n];
        let [first, _, _] = pushes(proto, n, vec![], &colors, &colors);
        let flags: Vec<u8> = first
            .iter()
            .filter_map(|op| match op {
                Op::F(b) if b[1] == 0x35 => Some(b[5]),
                _ => None,
            })
            .collect();
        assert_eq!(flags, [vec![0x0E; 10], vec![0x0A]].concat());
    }
}
