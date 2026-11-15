//! Razer misc devices (protocol learned from SignalRGB's Razer_Chroma_Light_Strip_Set.js,
//! Razer_Blackwidow_Ultimate_2013_2014_Keyboard.js, Razer_Nari_Headset_Controller.js and
//! Razer_Seiren_V3_Mic.js).

use super::common::{line, open_with, positioned};
use crate::hid::{Io, Proto, open_sibling};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x1532;
const FAMILY: &str = "razer";
const REPORT_LEN: usize = 91;
const STRIP_FRAME_LEN: usize = 381;
const STRIP_CHANNELS: usize = 3;
/// Three channels sized to the plugin's 240-LED device limit.
const STRIP_CHANNEL_LEDS: usize = 80;
const SEIREN_LEN: usize = 64;
const SEIREN_ID: u8 = 0x07;
const NARI_LEN: usize = 64;
const NARI_HEAD: [u8; 9] = [0xFF, 0x0A, 0x00, 0xFF, 0x04, 0x12, 0xF1, 0x05, 0x72];

/// The 91-byte Chroma feature report with its XOR checksum (bytes 3..89) at 89.
pub fn report(tid: u8, size: u8, class: u8, cmd: u8, args: &[u8]) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[2] = tid;
    r[6] = size;
    r[7] = class;
    r[8] = cmd;
    r[9..9 + args.len()].copy_from_slice(args);
    r[89] = r[3..89].iter().fold(0, |a, b| a ^ b);
    r
}

/// Light strip set takeover on interface 0, in the plugin's order.
pub fn strip_init() -> [[u8; REPORT_LEN]; 4] {
    [
        report(0x1F, 0x01, 0x00, 0x37, &[0x01]),
        report(0x9F, 0x02, 0x00, 0x47, &[0x01, 0x01]),
        report(0x1F, 0x02, 0x00, 0x84, &[0x00, 0x01]),
        report(0x1F, 0x02, 0x00, 0x04, &[0x03, 0x01]),
    ]
}

pub fn strip_release() -> [u8; REPORT_LEN] {
    report(0x1F, 0x01, 0x00, 0x37, &[0x00])
}

/// One channel's colours on interface 1; the last channel carries flag 0x84.
pub fn strip_frame(channel: u8, colors: &[Color]) -> Vec<u8> {
    let n = colors.len().clamp(1, (STRIP_FRAME_LEN - 6) / 3);
    let mut b = vec![0u8; STRIP_FRAME_LEN];
    b[1] = if channel == 2 { 0x84 } else { 0x04 };
    b[2] = channel;
    b[3] = channel;
    b[5] = (n - 1) as u8;
    for (i, c) in colors.iter().take(n).enumerate() {
        b[6 + i * 3..9 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

struct Strip<F: Io> {
    frames: F,
}

impl<F: Io> Proto for Strip<F> {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for r in strip_init() {
            io.feature(&r, REPORT_LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, _io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (ch, zone) in colors.chunks(STRIP_CHANNEL_LEDS).enumerate() {
            self.frames
                .feature(&strip_frame(ch as u8, zone), STRIP_FRAME_LEN)?;
            self.frames.sleep(10);
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(&strip_release(), REPORT_LEN);
    }
}

fn open_strip(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let frames = open_sibling(api, info, Some(1), Some(0x0C), Some(0x01))?;
    let mut dev = line(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Strip,
        STRIP_CHANNEL_LEDS,
    );
    dev.grid = [STRIP_CHANNEL_LEDS as u32, STRIP_CHANNELS as u32];
    dev.leds = (0..STRIP_CHANNELS)
        .flat_map(|r| {
            (0..STRIP_CHANNEL_LEDS).map(move |i| {
                [
                    (i as f32 + 0.5) / STRIP_CHANNEL_LEDS as f32,
                    (r as f32 + 0.5) / STRIP_CHANNELS as f32,
                ]
            })
        })
        .collect();
    open_with(api, info, FAMILY, dev, Strip { frames })
}

/// Mean of the three channels, rounded up, as the backlight level.
pub fn mean_level(c: Color) -> u8 {
    (c.0 as u16 + c.1 as u16 + c.2 as u16).div_ceil(3) as u8
}

pub fn brightness(level: u8) -> [u8; REPORT_LEN] {
    report(0xFF, 0x03, 0x03, 0x03, &[0x01, 0x04, level])
}

struct Backlight;

impl Proto for Backlight {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&brightness(mean_level(colors[0])), REPORT_LEN)
    }
}

fn open_backlight(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(&info.device_id(FAMILY), d.name, DeviceKind::Keyboard, 1);
    open_with(api, info, FAMILY, dev, Backlight)
}

pub fn nari_packet(c: Color) -> Vec<u8> {
    let mut b = NARI_HEAD.to_vec();
    b.extend_from_slice(&[c.0, c.1, c.2]);
    b
}

struct Nari;

impl Proto for Nari {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&nari_packet(colors[0]), NARI_LEN)?;
        io.sleep(30);
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(&nari_packet(Color(0, 0, 0)), NARI_LEN);
    }
}

fn open_nari(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(&info.device_id(FAMILY), d.name, DeviceKind::Other, 1);
    open_with(api, info, FAMILY, dev, Nari)
}

const SEIREN_POS: [(u8, u8); 10] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (0, 1),
    (1, 1),
    (2, 1),
    (3, 1),
    (4, 1),
    (5, 1),
];
/// Wire slot of each position above.
const SEIREN_SLOT: [usize; 10] = [7, 6, 8, 9, 2, 1, 3, 0, 4, 5];

/// The Seiren's 64-byte variant: report id 7, XOR of bytes 2..64 at 62.
pub fn seiren_report(body: &[u8]) -> [u8; SEIREN_LEN] {
    let mut r = [0u8; SEIREN_LEN];
    r[0] = SEIREN_ID;
    r[1..1 + body.len()].copy_from_slice(body);
    r[62] = r[2..].iter().fold(0, |a, b| a ^ b);
    r
}

pub fn seiren_init() -> [[u8; SEIREN_LEN]; 2] {
    [
        seiren_report(&[0x00, 0x1F, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04, 0x03]),
        seiren_report(&[
            0x00, 0x1F, 0x00, 0x00, 0x00, 0x06, 0x0F, 0x02, 0x00, 0x00, 0x08, 0x00, 0x01,
        ]),
    ]
}

pub fn seiren_frame(colors: &[Color]) -> [u8; SEIREN_LEN] {
    let n = SEIREN_SLOT.len();
    let mut body = vec![
        0x00,
        0x1F,
        0x00,
        0x00,
        0x00,
        (n * 3 + 5) as u8,
        0x0F,
        0x03,
        0x00,
        0x00,
        0x00,
        0x00,
        (n - 1) as u8,
    ];
    body.resize(13 + n * 3, 0);
    for (i, &slot) in SEIREN_SLOT.iter().enumerate() {
        let c = colors.get(i).copied().unwrap_or(Color(0, 0, 0));
        body[13 + slot * 3..16 + slot * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    seiren_report(&body)
}

struct Seiren;

impl Proto for Seiren {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for r in seiren_init() {
            io.feature(&r, SEIREN_LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&seiren_frame(colors), SEIREN_LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(&seiren_frame(&[]), SEIREN_LEN);
    }
}

fn open_seiren(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = positioned(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Other,
        6,
        2,
        &SEIREN_POS,
    );
    open_with(api, info, FAMILY, dev, Seiren)
}

type Open = fn(&HidApi, &Info, &Detector) -> Result<Box<dyn Backend>, String>;

const fn det(
    name: &'static str,
    pid: u16,
    interface: i32,
    page: Option<u16>,
    usage: Option<u16>,
    open: Open,
) -> Detector {
    Detector {
        name,
        vid: VID,
        pid,
        interface: Some(interface),
        usage_page: page,
        usage,
        open,
    }
}

pub static DETECTORS: &[Detector] = &[
    det(
        "Razer Chroma Light Strip Set",
        0x0F2C,
        0,
        Some(0x0C),
        Some(0x01),
        open_strip,
    ),
    det(
        "Razer Blackwidow Ultimate 2013/2014",
        0x011A,
        2,
        None,
        None,
        open_backlight,
    ),
    det("Razer Nari", 0x051C, 5, Some(0xFF00), Some(0x01), open_nari),
    det(
        "Razer Nari Essential",
        0x051F,
        5,
        Some(0xFF00),
        Some(0x01),
        open_nari,
    ),
    det(
        "Razer Nari Essential (Wireless)",
        0x051E,
        5,
        Some(0xFF00),
        Some(0x01),
        open_nari,
    ),
    det(
        "Razer Nari Ultimate",
        0x051A,
        5,
        Some(0xFF00),
        Some(0x01),
        open_nari,
    ),
    det(
        "Razer Seiren V3",
        0x056F,
        3,
        Some(0xFF53),
        Some(0x04),
        open_seiren,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;

    #[test]
    fn chroma_checksum_covers_3_to_88() {
        let r = report(0x1F, 0x01, 0x00, 0x37, &[0x01]);
        assert_eq!(r[..10], [0, 0, 0x1F, 0, 0, 0, 1, 0, 0x37, 1]);
        assert_eq!(r[89], 1 ^ 0x37 ^ 1);
    }

    #[test]
    fn strip_takeover_then_three_channels() {
        let mut cmd = Rec::default();
        let mut s = Strip {
            frames: Rec::default(),
        };
        s.start(&mut cmd).unwrap();
        let heads: Vec<_> = cmd
            .ops
            .iter()
            .map(|o| match o {
                Op::Feature(b) => (b.len(), b[2], b[6], b[8], b[9], b[10]),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            heads,
            [
                (91, 0x1F, 1, 0x37, 1, 0),
                (91, 0x9F, 2, 0x47, 1, 1),
                (91, 0x1F, 2, 0x84, 0, 1),
                (91, 0x1F, 2, 0x04, 3, 1),
            ]
        );
        let colors = vec![Color(1, 2, 3); STRIP_CHANNELS * STRIP_CHANNEL_LEDS];
        s.frame(&mut cmd, &colors).unwrap();
        assert_eq!(s.frames.ops.len(), 3);
        for (ch, op) in s.frames.ops.iter().enumerate() {
            let Op::Feature(b) = op else { panic!() };
            assert_eq!(b.len(), 381);
            let flag = if ch == 2 { 0x84 } else { 0x04 };
            assert_eq!(b[..9], [0, flag, ch as u8, ch as u8, 0, 79, 1, 2, 3]);
            assert_eq!(b[6 + 79 * 3..6 + 80 * 3], [1, 2, 3]);
            assert_eq!(b[6 + 80 * 3], 0);
        }
        s.stop(&mut cmd);
        assert_eq!(cmd.ops.last(), Some(&Op::Feature(strip_release().to_vec())));
        assert_eq!(strip_release()[9], 0);
    }

    #[test]
    fn backlight_from_mean_intensity() {
        assert_eq!(mean_level(Color(255, 255, 255)), 255);
        assert_eq!(mean_level(Color(1, 0, 0)), 1);
        let r = brightness(0x80);
        assert_eq!(r[..12], [0, 0, 0xFF, 0, 0, 0, 3, 3, 3, 1, 4, 0x80]);
        assert_eq!(r[89], 3 ^ 1 ^ 4 ^ 0x80);
    }

    #[test]
    fn nari_single_colour() {
        let mut io = Rec::default();
        Nari.frame(&mut io, &[Color(9, 8, 7)]).unwrap();
        let Op::Feature(b) = &io.ops[0] else { panic!() };
        assert_eq!(b.len(), 64);
        assert_eq!(
            b[..12],
            [0xFF, 0x0A, 0, 0xFF, 4, 0x12, 0xF1, 5, 0x72, 9, 8, 7]
        );
    }

    #[test]
    fn seiren_remaps_and_checks() {
        let colors: Vec<Color> = (0..10u8).map(|i| Color(i, i, i)).collect();
        let r = seiren_frame(&colors);
        assert_eq!(r[..14], [7, 0, 0x1F, 0, 0, 0, 35, 0x0F, 3, 0, 0, 0, 0, 9]);
        assert_eq!(r[14 + 7 * 3], 0);
        assert_eq!(r[14 + 6 * 3], 1);
        assert_eq!(r[14], 7);
        assert_eq!(r[62], r[2..62].iter().fold(0, |a, b| a ^ b));
        let init = seiren_init();
        assert_eq!(init[0][..10], [7, 0, 0x1F, 0, 0, 0, 2, 0, 4, 3]);
        assert_eq!(init[0][62], 0x1F ^ 2 ^ 4 ^ 3);
        assert_eq!(init[1][6..14], [6, 0x0F, 2, 0, 0, 8, 0, 1]);
    }
}
