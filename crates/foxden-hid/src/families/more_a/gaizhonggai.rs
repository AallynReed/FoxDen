//! Ported from OpenRGB Controllers/GaiZongGaiKeyboardController (GPL-2.0-or-later).

use std::f32::consts::PI;

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "gaizhonggai";
const VID: u16 = 0x3061;
const PRO_68: u16 = 0x4700;
const PRO_42: u16 = 0x4701;
const TOUCH_17: u16 = 0x4770;
const PRO_17: u16 = 0x4771;
const PRO_20: u16 = 0x4772;
const LIGHT_BOARD: u16 = 0x4710;
const HUB_GREEN: u16 = 0x4711;
const HUB_BLUE: u16 = 0x4712;
const DIAL: u16 = 0x4720;

const MAP_68: [&[u32]; 5] = [
    &[
        52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, NA, 66, 67,
    ],
    &[
        36, NA, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
    ],
    &[
        23, NA, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, NA, 35, NA, NA,
    ],
    &[
        10, NA, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, NA, 21, NA, 22, NA,
    ],
    &[0, 1, 2, NA, NA, NA, 3, NA, NA, NA, 4, 5, 6, NA, 7, 8, 9],
];

const MAP_42: [&[u32]; 4] = [
    &[41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30],
    &[29, 28, 27, 26, 25, 24, 23, 22, 21, 20, NA, 19],
    &[18, NA, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8],
    &[7, 6, 5, NA, 4, NA, NA, 3, NA, 2, 1, 0],
];

const MAP_DIAL: [&[u32]; 2] = [&[88, 85], &[87, 86]];

const MAP_17: [&[u32]; 5] = [
    &[84, 83, 82, 81],
    &[80, 79, 78, 77],
    &[76, 75, 74, NA],
    &[73, 72, 71, 68],
    &[70, NA, 69, NA],
];

const MAP_20: [&[u32]; 6] = [
    &[86, 87, 85, NA],
    &[84, 83, 82, 81],
    &[80, 79, 78, 77],
    &[76, 75, 74, NA],
    &[73, 72, 71, 68],
    &[70, NA, 69, NA],
];

const MAP_TOUCH: [&[u32]; 5] = [
    &[84, 83, 82, 81, NA],
    &[80, 79, 78, 77, 85],
    &[76, 75, 74, NA, 86],
    &[73, 72, 71, 68, 87],
    &[70, NA, 69, NA, NA],
];

fn packet(head: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

/// One write and one blocking read of 65 bytes.
fn ask(io: &mut dyn Io, head: &[u8], read: &mut [u8; 65]) -> Result<(), String> {
    io.write(&packet(head))?;
    io.read(read, BLOCKING_MS)?;
    Ok(())
}

/// Constructor for the light board: the 128-byte connection shape in three reads.
pub fn read_board_flash(io: &mut dyn Io) -> Result<[u8; 128], String> {
    let mut flash = [0u8; 128];
    let mut read = [0u8; 65];
    ask(io, &[0x85, 0, 60], &mut read)?;
    flash[..60].copy_from_slice(&read[3..63]);
    ask(io, &[0x85, 60, 60], &mut read)?;
    flash[60..120].copy_from_slice(&read[3..63]);
    ask(io, &[0x85, 120, 8], &mut read)?;
    flash[120..128].copy_from_slice(&read[3..11]);
    Ok(flash)
}

/// Constructor for the RGB hubs: channel lengths (command 0x87).
pub fn read_hub_flash(io: &mut dyn Io) -> Result<[u8; 16], String> {
    let mut read = [0u8; 65];
    ask(io, &[0x87], &mut read)?;
    let mut flash = [0u8; 16];
    flash.copy_from_slice(&read[2..18]);
    Ok(flash)
}

/// GetChannelLen.
fn channel_len(flash: &[u8], ch: usize) -> usize {
    ((flash[ch * 2] as usize) << 8) | flash[ch * 2 + 1] as usize
}

/// board_led_xy_self_call: walks the chained boards, giving each LED its matrix cell.
#[allow(clippy::too_many_arguments)]
fn board_walk(
    p: &[u8; 128],
    offset: &mut u8,
    cells: &mut Vec<(u8, u8)>,
    x: f32,
    y: f32,
    distance: f32,
    angle: f32,
) {
    if *offset >= 120 {
        return;
    }
    let temp = p[*offset as usize];
    let (led_distance, board_distance, range, step) = match temp >> 5 {
        0x07 => return,
        0x01 => (3.0, 4.33, 12u8, PI / 6.0),
        0x00 if temp >> 3 == 1 => (3.54, 5.0, 8, PI / 4.0),
        0x00 if temp >> 2 == 1 => (2.8, 2.9, 6, PI / 3.0),
        _ => (0.0, 0.0, 0, 0.0),
    };
    let new_x = x + angle.cos() * (distance + board_distance);
    let new_y = y + angle.sin() * (distance + board_distance);
    let mut new_angle = if angle > PI { angle - PI } else { angle + PI };
    let scale = p[120] as f32;
    for i in 1..range {
        new_angle -= step;
        if i & 1 == 1 {
            let fx = ((new_x + new_angle.cos() * led_distance) * scale * 0.01 + 0.5) / 1.0;
            let fy = ((new_y + new_angle.sin() * led_distance) * scale * 0.01 + 0.5) / -1.0;
            let cx = (fx as i16).wrapping_sub(p[122] as i8 as i16) as u8;
            let cy = (fy as i16).wrapping_sub(p[123] as i8 as i16) as u8;
            cells.push((cx, cy));
        } else if temp & (1 << (i / 2 - 1)) != 0 {
            *offset += 1;
            board_walk(p, offset, cells, new_x, new_y, board_distance, new_angle);
        }
    }
}

/// LightBoard_init: the LEDs of the board chain and their positions on its matrix.
pub fn light_board(p: &[u8; 128]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let angle = ((21 - p[121] as i32) % 12) as f32 * PI / 6.0;
    let mut cells = Vec::new();
    board_walk(p, &mut 0, &mut cells, 0.0, 0.0, 0.0, angle);
    let (w, h) = (p[124] as u32, p[125] as u32);
    let leds = cells
        .iter()
        .map(|&(x, y)| {
            if (x as u32) < w && (y as u32) < h {
                [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32]
            } else {
                [0.5, 0.5]
            }
        })
        .collect();
    (leds, [w.max(1), h.max(1)])
}

/// SendColors: how each product packs the colour buffer into reports.
pub fn reports(pid: u16, data: &[u8]) -> Vec<(Vec<u8>, bool)> {
    let at = |from: usize, len: usize| -> Vec<u8> {
        (from..from + len)
            .map(|i| data.get(i).copied().unwrap_or(0))
            .collect()
    };
    let one = |cmd: u8, from: usize, len: usize| {
        let mut h = vec![cmd];
        h.extend(at(from, len));
        packet(&h).to_vec()
    };
    match pid {
        PRO_68 => vec![
            (one(0x10, 0, 63), true),
            (one(0x11, 63, 63), true),
            (one(0x12, 126, 63), true),
            (one(0x13, 189, 15), false),
        ],
        PRO_42 => vec![(one(0x10, 0, 63), true), (one(0x11, 63, 63), false)],
        TOUCH_17 | PRO_20 => vec![(one(0x10, 68 * 3, 60), false)],
        PRO_17 => vec![(one(0x10, 68 * 3, 51), false)],
        DIAL => vec![(one(0x10, 85 * 3, 63), false)],
        _ => (0..32u8)
            .take_while(|&i| i as usize <= data.len() / 60)
            .map(|i| (one(i, i as usize * 60, 60), false))
            .collect(),
    }
}

struct Gai {
    pid: u16,
    /// Each exposed LED's index in OpenRGB's LED list.
    ids: Vec<u32>,
    total: usize,
}

impl Proto for Gai {
    /// DeviceUpdateLEDs: GRB per LED, then SendColors.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut data = vec![0u8; self.total * 3];
        for (&id, c) in self.ids.iter().zip(colors) {
            let i = id as usize * 3;
            data[i..i + 3].copy_from_slice(&[c.1, c.0, c.2]);
        }
        for (r, pause) in reports(self.pid, &data) {
            io.write(&r)?;
            if pause {
                io.sleep(2);
            }
        }
        Ok(())
    }

    /// The destructor restores the built-in effect.
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&packet(&[0xFF]));
    }
}

fn keys(map: &[&[u32]], total: usize) -> (Vec<u32>, Vec<[f32; 2]>, [u32; 2], usize) {
    let (ids, leds, grid) = mapped(map);
    (ids, leds, grid, total)
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let (ids, leds, grid, total, kind) = match info.pid {
        PRO_68 | PRO_42 | TOUCH_17 | PRO_17 | PRO_20 | DIAL => {
            let (ids, leds, grid, total) = match info.pid {
                PRO_68 => keys(&MAP_68, 68),
                PRO_42 => keys(&MAP_42, 42),
                TOUCH_17 => keys(&MAP_TOUCH, 88),
                PRO_17 => keys(&MAP_17, 85),
                PRO_20 => keys(&MAP_20, 88),
                _ => keys(&MAP_DIAL, 89),
            };
            let kind = if info.pid == DIAL {
                DeviceKind::Other
            } else {
                DeviceKind::Keyboard
            };
            (ids, leds, grid, total, kind)
        }
        LIGHT_BOARD => {
            let flash = read_board_flash(io.as_mut())?;
            let (leds, grid) = light_board(&flash);
            let n = leds.len();
            ((0..n as u32).collect(), leds, grid, n, DeviceKind::Other)
        }
        _ => {
            let flash = read_hub_flash(io.as_mut())?;
            let (channels, max) = if info.pid == HUB_GREEN {
                (4, 637)
            } else {
                (8, 633)
            };
            let counts: Vec<usize> = (0..channels)
                .map(|ch| channel_len(&flash, ch).min(max))
                .collect();
            let (leds, grid) = rows(&counts);
            let n = leds.len();
            ((0..n as u32).collect(), leds, grid, n, DeviceKind::Strip)
        }
    };
    if ids.is_empty() {
        return Err("no LEDs reported".into());
    }
    let dev = placed(info, FAMILY, d, kind, grid, leds);
    let proto = Gai {
        pid: info.pid,
        ids,
        total,
    };
    Ok(boxed(io, FAMILY, dev, proto))
}

pub static DETECTORS: [Detector; 9] = [
    det("GaiZhongGai 68+4 PRO", VID, PRO_68, 3, ANY, ANY, open),
    det("GaiZhongGai 42 PRO", VID, PRO_42, 3, ANY, ANY, open),
    det("GaiZhongGai Dial", VID, DIAL, 3, ANY, ANY, open),
    det(
        "GaiZhongGai LightBoard",
        VID,
        LIGHT_BOARD,
        3,
        ANY,
        ANY,
        open,
    ),
    det(
        "GaiZhongGai RGB HUB Green",
        VID,
        HUB_GREEN,
        3,
        ANY,
        ANY,
        open,
    ),
    det("GaiZhongGai RGB HUB Blue", VID, HUB_BLUE, 3, ANY, ANY, open),
    det(
        "GaiZhongGai 17+4+Touch PRO",
        VID,
        TOUCH_17,
        3,
        ANY,
        ANY,
        open,
    ),
    det("GaiZhongGai 17 PRO", VID, PRO_17, 3, ANY, ANY, open),
    det("GaiZhongGai 20 PRO", VID, PRO_20, 3, ANY, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn gai(pid: u16, map: &[&[u32]], total: usize) -> (Gai, usize) {
        let (ids, _, _) = mapped(map);
        let n = ids.len();
        (Gai { pid, ids, total }, n)
    }

    fn report(head: &[u8]) -> Op {
        w(packet(head).to_vec())
    }

    #[test]
    fn maps_hold_the_sent_leds() {
        let ids = |m: &[&[u32]]| mapped(m).0;
        assert_eq!(ids(&MAP_68), (0..68).collect::<Vec<_>>());
        assert_eq!(ids(&MAP_42), (0..42).collect::<Vec<_>>());
        assert_eq!(ids(&MAP_17), (68..85).collect::<Vec<_>>());
        assert_eq!(ids(&MAP_20), (68..88).collect::<Vec<_>>());
        assert_eq!(ids(&MAP_TOUCH), (68..88).collect::<Vec<_>>());
        assert_eq!(ids(&MAP_DIAL), (85..89).collect::<Vec<_>>());
    }

    // SendColors for the 68 PRO: 0x10..0x12 with 2 ms pauses, then 0x13 with 15 bytes
    #[test]
    fn pro68_frame() {
        let (proto, n) = gai(PRO_68, &MAP_68, 68);
        let colors: Vec<Color> = (0..68u8).map(|i| Color(i, 100 + i, 200)).collect();
        let [first, again, _] = pushes(proto, n, vec![], &colors, &colors);
        assert!(again.is_empty());
        let grb: Vec<u8> = colors.iter().flat_map(|c| [c.1, c.0, c.2]).collect();
        let head = |cmd: u8, from: usize, len: usize| {
            let mut h = vec![cmd];
            h.extend_from_slice(&grb[from..from + len]);
            report(&h)
        };
        assert_eq!(
            first,
            vec![
                head(0x10, 0, 63),
                Op::Sleep(2),
                head(0x11, 63, 63),
                Op::Sleep(2),
                head(0x12, 126, 63),
                Op::Sleep(2),
                head(0x13, 189, 15),
            ]
        );
    }

    // SendColors for the 42 PRO
    #[test]
    fn pro42_frame() {
        let (proto, n) = gai(PRO_42, &MAP_42, 42);
        let mut colors = vec![Color::BLACK; n];
        colors[41] = Color(1, 2, 3);
        let [first, _, _] = pushes(proto, n, vec![], &colors, &colors);
        let mut tail = vec![0x11];
        tail.extend(vec![0; 60]);
        tail.extend([2, 1, 3]);
        assert_eq!(first, vec![report(&[0x10]), Op::Sleep(2), report(&tail)]);
    }

    // SendColors for the keypads and dial: one report from their first LED
    #[test]
    fn keypad_and_dial_frames() {
        for (pid, map, total, len) in [
            (PRO_17, &MAP_17[..], 85, 51),
            (PRO_20, &MAP_20[..], 88, 60),
            (TOUCH_17, &MAP_TOUCH[..], 88, 60),
            (DIAL, &MAP_DIAL[..], 89, 63),
        ] {
            let (proto, n) = gai(pid, map, total);
            let colors = vec![Color(5, 6, 7); n];
            let [first, _, _] = pushes(proto, n, vec![], &colors, &colors);
            let mut h = vec![0x10];
            h.extend((0..len).map(|i| if i < n * 3 { [6, 5, 7][i % 3] } else { 0 }));
            assert_eq!(first, vec![report(&h)], "{pid:04x}");
        }
    }

    // Constructor for the RGB hubs, GetChannelLen
    #[test]
    fn hub_channel_lengths() {
        let mut reply = vec![0u8; 65];
        reply[2..6].copy_from_slice(&[0x00, 0x05, 0x01, 0x02]);
        let mut rec = Rec::with(vec![reply]);
        let flash = read_hub_flash(&mut rec).unwrap();
        assert_eq!(rec.take(), vec![report(&[0x87]), Op::Read(65)]);
        assert_eq!(channel_len(&flash, 0), 5);
        assert_eq!(channel_len(&flash, 1), 258);
        assert_eq!(channel_len(&flash, 2), 0);
    }

    // SendColors for the hubs: 60-byte reports numbered from 0, while i <= size / 60
    #[test]
    fn hub_frame() {
        let proto = Gai {
            pid: HUB_BLUE,
            ids: (0..21).collect(),
            total: 21,
        };
        let colors: Vec<Color> = (0..21u8).map(|i| Color(i, 0, 0)).collect();
        let [first, _, _] = pushes(proto, 21, vec![], &colors, &colors);
        let grb: Vec<u8> = colors.iter().flat_map(|c| [c.1, c.0, c.2]).collect();
        let mut p0 = vec![0];
        p0.extend_from_slice(&grb[..60]);
        let mut p1 = vec![1];
        p1.extend_from_slice(&grb[60..63]);
        assert_eq!(first, vec![report(&p0), report(&p1)]);
        let even = reports(HUB_GREEN, &[0u8; 60]);
        assert_eq!(even.len(), 2);
        assert_eq!(reports(HUB_GREEN, &[]).len(), 1);
        assert_eq!(reports(HUB_GREEN, &vec![0u8; 3000]).len(), 32);
    }

    // Constructor for the light board: three 0x85 reads into data_flash
    #[test]
    fn board_flash_reads() {
        let mut r1 = vec![0u8; 65];
        r1[3] = 0x20;
        let mut r3 = vec![0u8; 65];
        r3[3..9].copy_from_slice(&[100, 0, 0, 0, 9, 9]);
        let mut rec = Rec::with(vec![r1, vec![0u8; 65], r3]);
        let flash = read_board_flash(&mut rec).unwrap();
        assert_eq!(
            rec.take(),
            vec![
                report(&[0x85, 0, 60]),
                Op::Read(65),
                report(&[0x85, 60, 60]),
                Op::Read(65),
                report(&[0x85, 120, 8]),
                Op::Read(65),
            ]
        );
        assert_eq!(flash[0], 0x20);
        assert_eq!(&flash[120..126], &[100, 0, 0, 0, 9, 9]);
    }

    // LightBoard_init: a hexagon has 6 LEDs, children hang off its COM bits
    #[test]
    fn light_board_counts() {
        let mut p = [0u8; 128];
        p[120] = 100;
        p[124] = 9;
        p[125] = 9;
        p[0] = 0x20;
        assert_eq!(light_board(&p).0.len(), 6);
        p[0] = 0x20 | 0x01;
        p[1] = 0x08;
        assert_eq!(light_board(&p).0.len(), 10);
        p[1] = 0x04;
        assert_eq!(light_board(&p).0.len(), 9);
        p[0] = 0xE0;
        assert!(light_board(&p).0.is_empty());
    }

    // ~GaiZhongGaiKeyboardController writes 0xFF
    #[test]
    fn drop_restores_effect() {
        let rec = Rec::default();
        let (proto, n) = gai(PRO_42, &MAP_42, 42);
        let mut d = Driver::new(Box::new(rec.clone()), dev(n), "t", proto);
        foxden_core::Backend::push(&mut d, "t", &vec![Color::BLACK; n]).unwrap();
        rec.take();
        drop(d);
        assert_eq!(rec.take(), vec![report(&[0xFF])]);
    }
}
