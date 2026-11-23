// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::common::{Driver, Hid, Io, KEYBOARD, matrix, open_sibling, sum16};
use super::vulcan_layouts::{LAYOUTS, Layout};
use crate::registry::{Detector, Info};

const ROCCAT: u16 = 0x1E7D;
const TURTLE_BEACH: u16 = 0x10F5;

pub const VULCAN_100_AIMO: u16 = 0x307A;
pub const VULCAN_120_AIMO: u16 = 0x3098;
pub const VULCAN_TKL: u16 = 0x2FEE;
pub const VULCAN_PRO: u16 = 0x30F7;
pub const VULCAN_TKL_PRO: u16 = 0x311A;
pub const VULCAN_II: u16 = 0x2F4E;
pub const VULCAN_II_MINI: u16 = 0x2F09;
pub const VULCAN_II_MAX: u16 = 0x2EE2;
pub const PYRO: u16 = 0x314C;
pub const MAGMA: u16 = 0x3124;
pub const MAGMA_MINI: u16 = 0x69A0;
pub const TB_VULCAN_II: u16 = 0x501B;
pub const TB_VULCAN_II_TKL: u16 = 0x5023;
pub const TB_VULCAN_II_TKL_PRO: u16 = 0x5001;

const MODE_DIRECT: u8 = 0x0B;
const SPEED_DEFAULT: u8 = 0x06;
const BRIGHTNESS_DEFAULT: u8 = 0x45;

#[rustfmt::skip]
const MINI_SLOTS: [u8; 69] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
    18, 19, 20, 21, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33,
    34, 35, 36, 37, 38, 39, 40, 42, 43, 45, 46, 48, 49, 50, 51,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 64, 65, 66, 68,
    70, 71, 72, 73, 74, 75, 76,
];

/// The PIDs InitDeviceInfo and EnableDirect treat as the newer generation.
fn newer(pid: u16) -> bool {
    matches!(
        pid,
        PYRO | MAGMA
            | MAGMA_MINI
            | VULCAN_PRO
            | VULCAN_TKL_PRO
            | VULCAN_II
            | VULCAN_II_MINI
            | VULCAN_II_MAX
            | TB_VULCAN_II
            | TB_VULCAN_II_TKL_PRO
    )
}

/// (led interface, led page, control interface, control page) from DetectRoccatVulcanKeyboardControllers.
pub fn interfaces(pid: u16) -> (i32, u16, i32, u16) {
    match pid {
        TB_VULCAN_II_TKL_PRO => (4, 0xFF00, 1, 0x0001),
        p if newer(p) => (3, 0xFF00, 1, 0xFF01),
        _ => (3, 0x0001, 1, 0x000B),
    }
}

/// (packet length, report id) of InitDeviceInfo.
pub fn info_report(pid: u16) -> (usize, u8) {
    if newer(pid) { (9, 0x09) } else { (8, 0x0F) }
}

/// EnableDirect.
pub fn enable_direct(pid: u16, on: bool) -> Vec<u8> {
    if newer(pid) {
        vec![0x0E, 0x05, on as u8, 0x00, 0x00]
    } else {
        vec![0x15, 0x00, on as u8]
    }
}

fn wait_len(pid: u16) -> usize {
    match pid {
        PYRO | MAGMA | MAGMA_MINI | VULCAN_PRO | VULCAN_TKL_PRO => 4,
        _ => 3,
    }
}

/// SendMode with no colours, as Direct calls it; speed 0 and brightness 0 take the defaults.
pub fn mode_report(pid: u16, mode: u8, speed: u8, brightness: u8) -> Vec<u8> {
    let speed = if speed == 0 { SPEED_DEFAULT } else { speed };
    let brightness = if brightness == 0 {
        BRIGHTNESS_DEFAULT
    } else {
        brightness
    };
    let (len, proto): (usize, u8) = match pid {
        PYRO => (365, 2),
        MAGMA | MAGMA_MINI => (26, 2),
        VULCAN_PRO | VULCAN_TKL_PRO => (371, 2),
        VULCAN_II | TB_VULCAN_II => (377, 2),
        VULCAN_II_MAX => (542, 2),
        VULCAN_II_MINI => (332, 2),
        TB_VULCAN_II_TKL_PRO => (284, 2),
        _ => (443, 1),
    };
    let mut b = vec![0u8; len];
    let hdr = if len > 255 { 2 } else { 1 };
    b[0] = if proto == 1 { 0x0D } else { 0x11 };
    if hdr == 1 {
        b[1] = len as u8;
    } else {
        b[1] = (len % 256) as u8;
        b[2] = (len / 256) as u8;
    }
    let o = hdr + 1;
    b[o + 1] = mode;
    b[o + 2] = speed;
    if proto == 1 {
        b[o + 4] = brightness;
    } else {
        b[o + 3] = brightness;
    }
    if pid == VULCAN_II_MINI {
        b[o] = 0x02;
        b[o + 4] = 0xFB;
    }
    if pid == TB_VULCAN_II_TKL_PRO {
        b[o] = 0x03;
        b[o + 4] = 0x0B;
        b[o + 5] = 0x01;
    }
    let total = sum16(&b[..len - 2]);
    b[len - 2..].copy_from_slice(&total.to_le_bytes());
    b
}

/// SendColors: the 65-byte packets written to the LED interface.
pub fn color_packets(pid: u16, big_endian: bool, leds: &[(u8, Color)]) -> Vec<[u8; 65]> {
    let (len, col, proto): (usize, usize, u8) = match pid {
        MAGMA | MAGMA_MINI => (64, 5, 2),
        PYRO => (378, 1, 2),
        VULCAN_PRO | VULCAN_TKL_PRO => (384, 12, 2),
        VULCAN_II | TB_VULCAN_II => (396, 1, 2),
        VULCAN_II_MAX => (567, 1, 2),
        VULCAN_II_MINI => (345, 1, 2),
        TB_VULCAN_II_TKL_PRO => (320, 1, 2),
        _ => (436, 12, 1),
    };
    let n = len.div_ceil(64);
    let mut bufs = vec![[0u8; 65]; n];
    if proto > 1 {
        for (i, b) in bufs.iter_mut().enumerate() {
            b[1] = 0xA1;
            b[2] = i as u8 + 1;
        }
    } else {
        bufs[0][1] = 0xA1;
        bufs[0][2] = 0x01;
    }
    let hdr = if len > 255 { 4 } else { 3 };
    if hdr == 3 {
        bufs[0][3] = len as u8;
    } else if big_endian {
        bufs[0][3] = (len >> 8) as u8;
        bufs[0][4] = len as u8;
    } else {
        bufs[0][3] = len as u8;
        bufs[0][4] = (len >> 8) as u8;
    }
    let data = 64 - hdr;
    let mut put = |pos: usize, v: u8, by64: bool| {
        let (p, b) = if by64 {
            (pos / 64, pos % 64 + 1)
        } else {
            (pos / data, pos % data + hdr + 1)
        };
        if let Some(buf) = bufs.get_mut(p) {
            buf[b] = v;
        }
    };
    if pid == TB_VULCAN_II_TKL_PRO {
        let hw = leds.iter().map(|l| l.0 as usize).max().unwrap_or(0) + 1;
        for &(id, c) in leds {
            let id = id as usize;
            put(id, c.0, false);
            put(id + hw, c.1, false);
            put(id + 2 * hw, c.2, false);
        }
        return bufs;
    }
    for &(id, c) in leds {
        let mut idx = id as usize;
        if pid == VULCAN_II_MINI {
            match MINI_SLOTS.get(idx) {
                Some(&s) => idx = s as usize,
                None => continue,
            }
        }
        let (column, row) = (idx / col, idx % col);
        let first = column * 3 * col + row + if proto == 1 { hdr } else { 0 };
        for (k, v) in [c.0, c.1, c.2].into_iter().enumerate() {
            put(first + k * col, v, proto == 1);
        }
    }
    bufs
}

/// The layout SetupZones picks; a layout the table lacks falls back to the table's first.
pub fn layout(pid: u16, layout_type: u8) -> &'static Layout {
    let table = match pid {
        VULCAN_TKL | VULCAN_TKL_PRO | TB_VULCAN_II_TKL => "RoccatVulcanTKLLayouts",
        TB_VULCAN_II_TKL_PRO => "TurtleBeachVulcanIITKLProLayouts",
        VULCAN_PRO | PYRO => "RoccatPyroLayouts",
        VULCAN_II | TB_VULCAN_II => "RoccatVulcanIILayouts",
        VULCAN_II_MAX => "RoccatVulcanIIMaxLayouts",
        VULCAN_II_MINI => "RoccatVulcanIIMiniLayouts",
        MAGMA | MAGMA_MINI => "RoccatMagmaLayouts",
        _ => "RoccatVulcan120AimoLayouts",
    };
    let want = match layout_type {
        1..=3 => 2,
        6 => 6,
        _ => 0,
    };
    let mut rows = LAYOUTS.iter().filter(|l| l.table == table);
    LAYOUTS
        .iter()
        .find(|l| l.table == table && l.layout == want)
        .or_else(|| rows.next())
        .expect("every table has a layout")
}

/// What InitDeviceInfo reads: (firmware major, minor, layout type).
pub fn parse_info(pid: u16, first_len: usize, buf: &[u8]) -> (u8, u8, u8) {
    let layout = if matches!(pid, MAGMA | MAGMA_MINI) || first_len < 7 {
        0
    } else {
        buf[6]
    };
    (buf[2] / 100, buf[2] % 100, layout)
}

pub fn big_endian(pid: u16, major: u8, minor: u8) -> bool {
    pid != VULCAN_II_MINI && major >= 1 && minor >= 16
}

struct Vulcan {
    pid: u16,
    big_endian: bool,
    ids: &'static [u8],
    led: Box<dyn Io>,
}

fn wait_ready(io: &mut dyn Io, len: usize) {
    let mut buf = vec![0u8; len];
    buf[0] = 0x04;
    for i in 0..100 {
        if buf[1] == 1 {
            break;
        }
        if i != 0 {
            io.sleep(25);
        }
        let _ = io.get_feature(&mut buf);
    }
}

impl Driver for Vulcan {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&mode_report(self.pid, MODE_DIRECT, 0, 0))?;
        wait_ready(io, wait_len(self.pid));
        io.send_feature(&enable_direct(self.pid, true))?;
        wait_ready(io, wait_len(self.pid));
        Ok(())
    }

    fn frame(&mut self, _: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let leds: Vec<(u8, Color)> = self
            .ids
            .iter()
            .copied()
            .zip(colors.iter().copied())
            .collect();
        for p in color_packets(self.pid, self.big_endian, &leds) {
            self.led.write(&p)?;
        }
        let mut scratch = [0u8; 65];
        for _ in 0..64 {
            if !matches!(self.led.read(&mut scratch, 0), Ok(n) if n > 0) {
                break;
            }
        }
        let _ = self.led.read(&mut scratch, 20);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let pid = info.pid;
    let (led_if, led_page, ctrl_if, ctrl_page) = interfaces(pid);
    let mut ctrl = open_sibling(api, info, |s| {
        s.interface == ctrl_if && s.usage_page == ctrl_page
    })?;
    let led = open_sibling(api, info, |s| {
        s.interface == led_if && s.usage_page == led_page
    })?;
    let (len, id) = info_report(pid);
    let mut buf = vec![0u8; len];
    buf[0] = id;
    let first = Io::get_feature(&mut ctrl, &mut buf).unwrap_or(0);
    let _ = Io::get_feature(&mut ctrl, &mut buf);
    let (major, minor, layout_type) = parse_info(pid, first, &buf);
    let lay = layout(pid, layout_type);
    let dev = matrix(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        (lay.rows, lay.cols),
        lay.matrix,
        lay.ids.len(),
    );
    let drv = Vulcan {
        pid,
        big_endian: big_endian(pid, major, minor),
        ids: lay.ids,
        led: Box::new(led),
    };
    Ok(Hid::boxed("roccat vulcan", ctrl, drv, dev))
}

macro_rules! ip {
    ($name:expr, $vid:expr, $pid:expr, $page:expr) => {
        Detector {
            name: $name,
            vid: $vid,
            pid: $pid,
            interface: Some(1),
            usage_page: Some($page),
            usage: None,
            open,
        }
    };
}

pub static DETECTORS: [Detector; 14] = [
    ip!("Roccat Magma", ROCCAT, MAGMA, 0xFF01),
    ip!("Roccat Magma Mini", ROCCAT, MAGMA_MINI, 0xFF01),
    ip!("Roccat Pyro", ROCCAT, PYRO, 0xFF01),
    ip!("Roccat Vulcan 100 Aimo", ROCCAT, VULCAN_100_AIMO, 11),
    ip!("Roccat Vulcan 120-Series Aimo", ROCCAT, VULCAN_120_AIMO, 11),
    ip!("Roccat Vulcan TKL", ROCCAT, VULCAN_TKL, 11),
    ip!("Roccat Vulcan Pro", ROCCAT, VULCAN_PRO, 0xFF01),
    ip!("Roccat Vulcan TKL Pro", ROCCAT, VULCAN_TKL_PRO, 0xFF01),
    ip!("Roccat Vulcan II", ROCCAT, VULCAN_II, 0xFF01),
    ip!("Roccat Vulcan II Max", ROCCAT, VULCAN_II_MAX, 0xFF01),
    ip!("Roccat Vulcan II Mini", ROCCAT, VULCAN_II_MINI, 0xFF01),
    ip!("Turtle Beach Vulcan II", TURTLE_BEACH, TB_VULCAN_II, 0xFF01),
    ip!(
        "Turtle Beach Vulcan II TKL",
        TURTLE_BEACH,
        TB_VULCAN_II_TKL,
        11
    ),
    ip!(
        "Turtle Beach Vulcan II TKL Pro",
        TURTLE_BEACH,
        TB_VULCAN_II_TKL_PRO,
        0x0001
    ),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Mock, Op, pad};
    use super::*;

    // RoccatVulcanKeyboardController::SendMode(0x0B, 0, 0x45, {}) on the 120 Aimo (protocol 1).
    #[test]
    fn mode_report_protocol_1() {
        let b = mode_report(VULCAN_120_AIMO, MODE_DIRECT, 0, 0);
        assert_eq!(b.len(), 443);
        assert_eq!(
            b[..9],
            [0x0D, 0xBB, 0x01, 0x00, 0x0B, 0x06, 0x00, 0x45, 0x00]
        );
        // 0x0D + 0xBB + 0x01 + 0x0B + 0x06 + 0x45 = 0x11F
        assert_eq!(b[441..], [0x1F, 0x01]);
        assert!(b[9..441].iter().all(|&x| x == 0));
    }

    // SendMode on the protocol 2 parts, including the Mini and TKL Pro overrides.
    #[test]
    fn mode_report_protocol_2() {
        let b = mode_report(MAGMA, MODE_DIRECT, 0, 0x45);
        assert_eq!(b.len(), 26);
        assert_eq!(b[..8], [0x11, 26, 0x00, 0x0B, 0x06, 0x45, 0x00, 0x00]);
        assert_eq!(b[24..], (0x11u16 + 26 + 0x0B + 0x06 + 0x45).to_le_bytes());
        let m = mode_report(VULCAN_II_MINI, MODE_DIRECT, 0, 0x45);
        assert_eq!(
            m[..9],
            [0x11, 0x4C, 0x01, 0x02, 0x0B, 0x06, 0x45, 0xFB, 0x00]
        );
        let t = mode_report(TB_VULCAN_II_TKL_PRO, MODE_DIRECT, 0, 0x45);
        assert_eq!(
            t[..9],
            [0x11, 0x1C, 0x01, 0x03, 0x0B, 0x06, 0x45, 0x0B, 0x01]
        );
    }

    // EnableDirect for both generations.
    #[test]
    fn enable_direct_reports() {
        assert_eq!(
            enable_direct(PYRO, true),
            vec![0x0E, 0x05, 0x01, 0x00, 0x00]
        );
        assert_eq!(enable_direct(VULCAN_TKL, true), vec![0x15, 0x00, 0x01]);
    }

    // SendColors, protocol 1: 7 packets, header A1 01 B4 01, colour columns of 12.
    #[test]
    fn color_packets_protocol_1() {
        let p = color_packets(
            VULCAN_120_AIMO,
            false,
            &[(0x00, Color(1, 2, 3)), (0x0D, Color(4, 5, 6))],
        );
        assert_eq!(p.len(), 7);
        assert_eq!(p[0][..5], [0x00, 0xA1, 0x01, 0xB4, 0x01]);
        assert_eq!((p[0][5], p[0][17], p[0][29]), (1, 2, 3));
        // id 13: column 1, row 1 -> offset 36 + 1 + 4 = 41
        assert_eq!((p[0][42], p[0][54], p[1][2]), (4, 5, 6));
        assert!(p[1..].iter().all(|b| b[1] == 0));
    }

    // SendColors, protocol 2 with a big-endian length (firmware 1.16+).
    #[test]
    fn color_packets_protocol_2() {
        let p = color_packets(
            VULCAN_II,
            true,
            &[(0x00, Color(1, 2, 3)), (20, Color(4, 5, 6))],
        );
        assert_eq!(p.len(), 7);
        assert_eq!(p[0][..5], [0x00, 0xA1, 0x01, 0x01, 0x8C]);
        assert_eq!(p[6][..3], [0x00, 0xA1, 0x07]);
        assert_eq!(p[0][5..8], [1, 2, 3]);
        // id 20 -> offset 60 -> packet 1, byte 0 + 4 + 1
        assert_eq!(p[1][5..8], [4, 5, 6]);
        let le = color_packets(VULCAN_II, false, &[]);
        assert_eq!(le[0][3..5], [0x8C, 0x01]);
        let magma = color_packets(MAGMA, false, &[(1, Color(7, 8, 9))]);
        assert_eq!(magma.len(), 1);
        assert_eq!(magma[0][..4], [0x00, 0xA1, 0x01, 64]);
        assert_eq!((magma[0][5], magma[0][10], magma[0][15]), (7, 8, 9));
    }

    // SendColors on the Vulcan II Mini goes through the slot map.
    #[test]
    fn mini_uses_slot_map() {
        let p = color_packets(
            VULCAN_II_MINI,
            false,
            &[(17, Color(1, 2, 3)), (69, Color(9, 9, 9))],
        );
        // id 17 -> slot 18 -> offset 54 -> packet 0, byte 54 + 4 + 1
        assert_eq!(p[0][59..62], [1, 2, 3]);
        assert_eq!(p.iter().flatten().filter(|&&b| b == 9).count(), 0);
    }

    // SendColors on the Turtle Beach TKL Pro: planar R, G, B blocks sized by the highest id.
    #[test]
    fn tkl_pro_is_planar() {
        let p = color_packets(
            TB_VULCAN_II_TKL_PRO,
            false,
            &[(0, Color(1, 2, 3)), (9, Color(4, 5, 6))],
        );
        assert_eq!(p[0][5], 1);
        assert_eq!(p[0][15], 2);
        assert_eq!(p[0][25], 3);
        assert_eq!((p[0][14], p[0][24], p[0][34]), (4, 5, 6));
    }

    // InitDeviceInfo, IsBigEndianDirectMode and SetupZones' layout choice.
    #[test]
    fn device_info_and_layout() {
        let buf = [0x09, 0x00, 116, 0, 0, 0, 0x01, 0, 0];
        assert_eq!(parse_info(VULCAN_II_MAX, 9, &buf), (1, 16, 1));
        assert_eq!(parse_info(MAGMA, 9, &buf).2, 0);
        assert_eq!(parse_info(VULCAN_II_MAX, 5, &buf).2, 0);
        assert!(big_endian(VULCAN_II_MAX, 1, 16));
        assert!(!big_endian(VULCAN_II_MINI, 1, 16));
        assert!(!big_endian(VULCAN_II_MAX, 2, 5));
        assert_eq!(layout(VULCAN_120_AIMO, 0).ids.len(), 104);
        assert_eq!(layout(VULCAN_120_AIMO, 1).ids.len(), 105);
        assert_eq!(layout(VULCAN_II_MAX, 6).ids.len(), 151);
        assert_eq!(layout(VULCAN_II_MAX, 0).ids.len(), 148);
        assert_eq!(layout(MAGMA, 2).ids.len(), 5);
        assert_eq!(interfaces(TB_VULCAN_II_TKL_PRO), (4, 0xFF00, 1, 0x0001));
        assert_eq!(interfaces(VULCAN_TKL), (3, 0x0001, 1, 0x000B));
    }

    // RGBController_RoccatVulcanKeyboard::DeviceUpdateMode then DeviceUpdateLEDs, as SetupZones runs them.
    #[test]
    fn setup_then_frame_on_two_interfaces() {
        let ctrl = Mock::default();
        let led = Mock::default();
        ctrl.reply(vec![0x04, 0x01, 0x00]);
        ctrl.reply(vec![0x04, 0x01, 0x00]);
        let lay = layout(MAGMA, 0);
        let drv = Vulcan {
            pid: MAGMA,
            big_endian: false,
            ids: lay.ids,
            led: Box::new(led.clone()),
        };
        let dev = matrix(
            "k".into(),
            "K",
            DeviceKind::Keyboard,
            KEYBOARD,
            (1, 5),
            lay.matrix,
            5,
        );
        let mut b = Hid::new("t", Box::new(ctrl.clone()), drv, dev);
        b.push("k", &[Color(7, 8, 9); 5]).unwrap();
        assert_eq!(
            ctrl.take(),
            vec![
                Op::Feature(mode_report(MAGMA, MODE_DIRECT, 0, 0)),
                Op::GetFeature(pad(&[0x04], 4)),
                Op::Feature(enable_direct(MAGMA, true)),
                Op::GetFeature(pad(&[0x04], 4)),
            ]
        );
        let ids: Vec<(u8, Color)> = lay.ids.iter().map(|&i| (i, Color(7, 8, 9))).collect();
        assert_eq!(
            led.take(),
            vec![
                Op::Write(color_packets(MAGMA, false, &ids)[0].to_vec()),
                Op::Read(65, 0),
                Op::Read(65, 20),
            ]
        );
    }
}
