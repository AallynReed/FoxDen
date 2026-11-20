//! Ported from OpenRGB Controllers/WootingKeyboardController (GPL-2.0-or-later).

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, NA, any, at, backend, cell_pos, device, on_p, open_io};
use crate::registry::{Detector, Info};

const FRAME: usize = 6 * 21;
const RAW_COLORS: u8 = 11;
const COLOR_INIT: u8 = 33;
const PAGE_V2: u16 = 0x1337;
const PAGE_V3: u16 = 0xFF55;
const OLD_VID: u16 = 0x03EB;
const NEW_VID: u16 = 0x31E3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Tkl,
    Full,
    Sixty,
    ThreePad,
    Eighty,
}

/// `DetectWootingControllers`: the layout from the product id.
fn kind(pid: u16) -> Kind {
    match pid {
        0xFF01 => return Kind::Tkl,
        0xFF02 => return Kind::Full,
        _ => {}
    }
    match pid & 0xFFF0 {
        0xFF00 | 0x1100 => Kind::Tkl,
        0x1300 | 0x1310 | 0x1320 | 0x1340 => Kind::Sixty,
        0x1400 => Kind::Eighty,
        0x1510 => Kind::ThreePad,
        _ => Kind::Full,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Proto {
    V1,
    V2,
    V3,
}

/// `DetectWootingControllers`: the protocol from the vendor and the config usage page.
fn proto(vid: u16, page: u16) -> Option<Proto> {
    match page {
        PAGE_V2 if vid == OLD_VID => Some(Proto::V1),
        PAGE_V2 => Some(Proto::V2),
        PAGE_V3 => Some(Proto::V3),
        _ => None,
    }
}

#[rustfmt::skip]
const MAP_FULL: [u32; 6 * 21] = [
      0,  NA,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,  16,  17,  18,  19,  20,
     21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,  33,  34,  35,  36,  37,  38,  39,  40,  41,
     42,  43,  44,  45,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,  56,  57,  58,  59,  60,  61,  62,
     63,  64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,  NA,  NA,  NA,  80,  81,  82,  NA,
     84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,  NA,  97,  NA,  99,  NA, 101, 102, 103, 104,
    105, 106, 107,  NA,  NA,  NA, 111,  NA,  NA,  NA, 115, 116, 117, 118, 119, 120, 121,  NA, 123, 124,  NA,
];

#[rustfmt::skip]
const MAP_TKL: [u32; 6 * 17] = [
      0,  NA,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,  16,
     21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,  33,  34,  35,  36,  37,
     42,  43,  44,  45,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,  56,  57,  58,
     63,  64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,  NA,  NA,  NA,
     84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,  NA,  97,  NA,  99,  NA,
    105, 106, 107,  NA,  NA,  NA, 111,  NA,  NA,  NA, 115, 116, 117, 118, 119, 120, 121,
];

#[rustfmt::skip]
const MAP_80: [u32; 6 * 17] = [
      0,   1,   2,   3,   4,   5,   6,   7,   8,  NA,  10,  11,  12,  13,  14,  15,  16,
     21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,  33,  34,  35,  36,  37,
     42,  43,  44,  45,  46,  47,  48,  49,  50,  51,  52,  53,  54,  NA,  56,  57,  58,
     63,  64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  NA,  77,  NA,  NA,
     84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,  NA,  97,  98,  99,  NA,
    105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,  NA, 119, 120, 121,
];

#[rustfmt::skip]
const MAP_60: [u32; 5 * 14] = [
     21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,  33,  34,
     42,  43,  44,  45,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,
     63,  64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,
     84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,  NA,  97,
    105, 106, 107,  NA, 109, 110, 111, 112, 113,  NA, 115, 116, 117, 118,
];

#[rustfmt::skip]
const MAP_3PAD: [u32; 5 * 7] = [
      0,  NA,   2,  NA,   4,  NA,   6,
     21,  NA,  NA,  NA,  NA,  NA,  27,
     42,  43,  NA,  45,  NA,  47,  48,
     63,  NA,  NA,  NA,  NA,  NA,  69,
     NA,  85,  86,  NA,  88,  89,  NA,
];

/// `RGBController_WootingKeyboard::SetupZones`: the layout's map and width.
fn layout(kind: Kind) -> (&'static [u32], usize) {
    match kind {
        Kind::Tkl => (&MAP_TKL, 17),
        Kind::Eighty => (&MAP_80, 17),
        Kind::Sixty => (&MAP_60, 14),
        Kind::ThreePad => (&MAP_3PAD, 7),
        Kind::Full => (&MAP_FULL, 21),
    }
}

/// One LED per used cell, row by row: its framebuffer index and its position.
fn keys(kind: Kind) -> Vec<(usize, [f32; 2])> {
    let (map, width) = layout(kind);
    let height = map.len() / width;
    map.iter()
        .enumerate()
        .filter(|(_, v)| **v != NA)
        .map(|(cell, v)| (*v as usize, cell_pos(cell, width, height)))
        .collect()
}

/// `WootingKeyboardController::wooting_usb_send_feature`.
fn feature(command: u8, p: [u8; 4]) -> Vec<u8> {
    vec![0, 0xD0, 0xDA, command, p[3], p[2], p[1], p[0]]
}

#[rustfmt::skip]
const V1_INDEX: [u8; 6 * 21] = [
      0, 255,  11,  12,  23,  24,  36,  47,  85,  84,  49,  48,  59,  61,  73,  81,  80, 113, 114, 115, 116,
      2,   1,  14,  13,  26,  25,  35,  38,  37,  87,  86,  95,  51,  63,  75,  72,  74,  96,  97,  98,  99,
      3,   4,  15,  16,  27,  28,  39,  42,  40,  88,  89,  52,  53,  71,  76,  83,  77, 102, 103, 104, 100,
      5,   6,  17,  18,  29,  30,  41,  46,  44,  90,  93,  54,  57,  65, 255, 255, 255, 105, 106, 107, 255,
      9,   8,  19,  20,  31,  34,  32,  45,  43,  91,  92,  55, 255,  66, 255,  78, 255, 108, 109, 110, 101,
     10,  22,  21, 255, 255, 255,  33, 255, 255, 255,  94,  58,  67,  68,  70,  79,  82, 255, 111, 112, 255,
];

const PWM: [u8; 24] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x30, 0x31, 0x32, 0x33,
    0x34, 0x35, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
];

fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
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

/// `WootingV1KeyboardController::SendDirect`: five 129-byte raw register writes.
fn v1_reports(frame: &[Color]) -> Vec<Vec<u8>> {
    let mut bufs = [[0u8; 96]; 5];
    for (index, &led) in V1_INDEX.iter().enumerate() {
        if led > 116 {
            continue;
        }
        let part = match led {
            96.. => 4,
            72.. => 3,
            48.. => 2,
            24.. => 1,
            _ => 0,
        };
        let at_ = PWM[led as usize % 24] as usize;
        let c = at(frame, index);
        bufs[part][at_] = c.0;
        bufs[part][at_ + 0x10] = c.1;
        bufs[part][at_ + 0x20] = c.2;
    }
    let addr = [(0, 0), (0, 96), (1, 0), (1, 96), (2, 0)];
    bufs.iter()
        .zip(addr)
        .map(|(buf, (slave, reg))| {
            let mut r = vec![0u8; 129];
            r[..6].copy_from_slice(&[0, 0xD0, 0xDA, RAW_COLORS, slave, reg]);
            r[6..102].copy_from_slice(buf);
            let crc = crc16_ccitt(&r[..127]);
            r[127] = crc as u8;
            r[128] = (crc >> 8) as u8;
            r
        })
        .collect()
}

fn rgb565(c: Color) -> u16 {
    ((c.0 as u16 & 0xF8) << 8) | ((c.1 as u16 & 0xFC) << 3) | (c.2 as u16 >> 3)
}

/// `WootingV2KeyboardController::SendDirect` and the V3 one: RGB565 words after a header.
fn v23_report(proto: Proto, frame: &[Color]) -> Vec<u8> {
    let (len, head) = match proto {
        Proto::V3 => (2046, [4, 0xD1, 0xDA, RAW_COLORS]),
        _ => (257, [0, 0xD0, 0xDA, RAW_COLORS]),
    };
    let mut r = vec![0u8; len];
    r[..4].copy_from_slice(&head);
    for i in 0..FRAME {
        let w = rgb565(at(frame, i));
        r[4 + i * 2] = w as u8;
        r[5 + i * 2] = (w >> 8) as u8;
    }
    r
}

struct Wooting {
    proto: Proto,
    slots: Vec<usize>,
}

impl Driver for Wooting {
    // WootingKeyboardController::SendInitialize, run by every controller's constructor.
    fn setup(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&feature(COLOR_INIT, [0; 4]))?;
        io.sleep(Duration::from_millis(50));
        Ok(())
    }

    // RGBController_WootingKeyboard::DeviceUpdateLEDs: LEDs into the 6x21 framebuffer, then SendDirect.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut frame = vec![Color::default(); FRAME];
        for (led, &slot) in self.slots.iter().enumerate() {
            frame[slot] = at(colors, led);
        }
        match self.proto {
            Proto::V1 => v1_reports(&frame).iter().try_for_each(|r| io.write(r)),
            p => io.write(&v23_report(p, &frame)),
        }
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let proto = proto(info.vid, info.usage_page).ok_or("not the lighting interface")?;
    let kind = kind(info.pid);
    let (slots, leds): (Vec<usize>, Vec<[f32; 2]>) = keys(kind).into_iter().unzip();
    let (map, width) = layout(kind);
    let io = open_io(api, info)?;
    let dev = device(
        info.device_id("wooting"),
        d.name,
        DeviceKind::Keyboard,
        [width as u32, (map.len() / width) as u32],
        leds,
    );
    Ok(backend(
        io,
        dev,
        Wooting { proto, slots },
        "wooting keyboard",
    ))
}

/// `REGISTER_WOOTING_DETECTOR`: the none, XInput and DirectInput product ids.
const fn three(name: &'static str, pid: u16) -> [Detector; 3] {
    [
        any(name, NEW_VID, pid | 2, open),
        any(name, NEW_VID, pid, open),
        any(name, NEW_VID, pid | 1, open),
    ]
}

pub(super) static LEGACY: [Detector; 2] = [
    on_p("Wooting One (Legacy)", OLD_VID, 0xFF01, PAGE_V2, open),
    on_p("Wooting Two (Legacy)", OLD_VID, 0xFF02, PAGE_V2, open),
];

pub(super) static CURRENT: [[Detector; 3]; 11] = [
    three("Wooting One", 0x1100),
    three("Wooting Two", 0x1200),
    three("Wooting Two Lekker Edition", 0x1210),
    three("Wooting Two HE", 0x1220),
    three("Wooting Two HE (ARM)", 0x1230),
    three("Wooting 60HE", 0x1300),
    three("Wooting 60HE (ARM)", 0x1310),
    three("Wooting 60HE+", 0x1320),
    three("Wooting 60HEv2", 0x1340),
    three("Wooting 80HE", 0x1400),
    three("Wooting UwU RGB", 0x1510),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op, ms};
    use super::*;

    // DetectWootingControllers: type from the product id, protocol from the usage page
    #[test]
    fn detect_picks_layout_and_protocol() {
        assert_eq!(kind(0xFF01), Kind::Tkl);
        assert_eq!(kind(0xFF02), Kind::Full);
        assert_eq!(kind(0x1102), Kind::Tkl);
        assert_eq!(kind(0x1231), Kind::Full);
        assert_eq!(kind(0x1342), Kind::Sixty);
        assert_eq!(kind(0x1400), Kind::Eighty);
        assert_eq!(kind(0x1511), Kind::ThreePad);
        assert_eq!(kind(0x1600), Kind::Full);
        assert_eq!(proto(OLD_VID, PAGE_V2), Some(Proto::V1));
        assert_eq!(proto(NEW_VID, PAGE_V2), Some(Proto::V2));
        assert_eq!(proto(NEW_VID, PAGE_V3), Some(Proto::V3));
        assert_eq!(proto(NEW_VID, 0x0001), None);
    }

    // RGBController_WootingKeyboard::SetupZones: LED counts per layout
    #[test]
    fn key_counts() {
        let n = |k| keys(k).len();
        assert_eq!(n(Kind::Full), 110);
        assert_eq!(n(Kind::Tkl), 89);
        assert_eq!(n(Kind::Eighty), 94);
        assert_eq!(n(Kind::Sixty), 67);
        assert_eq!(n(Kind::ThreePad), 17);
        assert_eq!(keys(Kind::Sixty)[0], (21, [0.5 / 14.0, 0.1]));
    }

    // WootingKeyboardController::SendInitialize: init feature, then 50 ms
    #[test]
    fn setup_sends_color_init() {
        let mut fake = Fake::default();
        let mut w = Wooting {
            proto: Proto::V2,
            slots: vec![],
        };
        w.setup(&mut fake).unwrap();
        assert_eq!(
            fake.ops(),
            vec![Op::Feature(vec![0, 0xD0, 0xDA, 0x21, 0, 0, 0, 0]), ms(50)]
        );
    }

    #[test]
    fn crc_is_ccitt_zero_init() {
        assert_eq!(crc16_ccitt(b"123456789"), 0x31C3);
    }

    // WootingV2KeyboardController::SendDirect: 257 bytes of RGB565 little-endian
    #[test]
    fn v2_frame() {
        let mut fake = Fake::default();
        let mut w = Wooting {
            proto: Proto::V2,
            slots: keys(Kind::Sixty).into_iter().map(|k| k.0).collect(),
        };
        let mut colors = vec![Color::default(); 67];
        colors[0] = Color(0xFF, 0x80, 0x10);
        w.frame(&mut fake, &colors).unwrap();
        let ops = fake.ops();
        let Op::Write(r) = &ops[0] else { panic!() };
        assert_eq!(r.len(), 257);
        assert_eq!(r[..4], [0, 0xD0, 0xDA, 0x0B]);
        // LED 0 is framebuffer slot 21: 0xF800 | 0x0400 | 0x0002
        assert_eq!(r[4 + 42..4 + 44], [0x02, 0xFC]);
        assert!(r[4..4 + 42].iter().all(|&b| b == 0));
    }

    // WootingV3KeyboardController::SendDirect: 2046 bytes after report id 4
    #[test]
    fn v3_frame() {
        let r = v23_report(Proto::V3, &[Color(8, 4, 8)]);
        assert_eq!(r.len(), 2046);
        assert_eq!(r[..6], [4, 0xD1, 0xDA, 0x0B, 0x21, 0x08]);
    }

    // WootingV1KeyboardController::SendDirect + wooting_usb_send_buffer
    #[test]
    fn v1_frame() {
        let mut frame = vec![Color::default(); FRAME];
        frame[0] = Color(1, 2, 3); // index 0 -> led 0 -> part 0, pwm 0x00
        frame[22] = Color(4, 5, 6); // index 22 -> led 1 -> part 0, pwm 0x01
        frame[17] = Color(7, 8, 9); // index 17 -> led 113 -> part 4, pwm[17] 0x35
        frame[1] = Color(9, 9, 9); // NA, skipped
        let r = v1_reports(&frame);
        assert_eq!(r.len(), 5);
        let mut p0 = vec![0u8; 129];
        p0[..6].copy_from_slice(&[0, 0xD0, 0xDA, 11, 0, 0]);
        p0[6] = 1;
        p0[6 + 0x10] = 2;
        p0[6 + 0x20] = 3;
        p0[7] = 4;
        p0[7 + 0x10] = 5;
        p0[7 + 0x20] = 6;
        let crc = crc16_ccitt(&p0[..127]);
        p0[127] = crc as u8;
        p0[128] = (crc >> 8) as u8;
        assert_eq!(r[0], p0);
        assert_eq!(r[1][..6], [0, 0xD0, 0xDA, 11, 0, 96]);
        assert_eq!(r[3][..6], [0, 0xD0, 0xDA, 11, 1, 96]);
        assert_eq!(r[4][..6], [0, 0xD0, 0xDA, 11, 2, 0]);
        assert_eq!([r[4][6 + 0x35], r[4][6 + 0x45], r[4][6 + 0x55]], [7, 8, 9]);
        assert_eq!(r[4][6..102].iter().filter(|&&b| b != 0).count(), 3);
    }
}
