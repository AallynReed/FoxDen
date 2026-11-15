//! Ported from OpenRGB Controllers/LenovoControllers and Controllers/LenovoMotherboardController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::common::{BOARD, Driver, Hid, Io, KEYBOARD, NA, line, matrix};
use super::lenovo_zones::*;
use crate::registry::{Detector, Info};

const ITE: u16 = 0x048D;
const LENOVO: u16 = 0x17EF;

/// LenovoMotherboardController::SetMode(zone, Static, brightness 4, speed 1, colour).
pub fn board_reports(zone: u8, c: Color) -> [[u8; 64]; 2] {
    let mut a = [0u8; 64];
    a[..8].copy_from_slice(&[0xCC, zone, 0x01, 0x01, 0x04, c.0, c.1, c.2]);
    let mut b = [0u8; 64];
    b[..3].copy_from_slice(&[0xCC, 0x28, 0x06]);
    b[33..40].copy_from_slice(&[zone, 0x01, 0x01, 0x04, c.0, c.1, c.2]);
    [a, b]
}

struct Board;

impl Driver for Board {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (zone, c) in [(0x12, colors[0]), (0x11, colors[1])] {
            for r in board_reports(zone, c) {
                io.send_feature(&r)?;
            }
        }
        Ok(())
    }
}

fn open_board(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("lenovo"),
        d.name,
        DeviceKind::Motherboard,
        BOARD,
        2,
    );
    Ok(Hid::boxed(
        "lenovo motherboard",
        info.open(api)?,
        Board,
        dev,
    ))
}

/// Lenovo4ZoneUSBController::setMode with KeyboardState in static effect, slowest speed, high brightness.
pub fn four_zone_report(colors: &[Color]) -> [u8; 33] {
    let mut b = [0u8; 33];
    b[..5].copy_from_slice(&[0xCC, 0x16, 0x01, 0x01, 0x02]);
    for (i, c) in colors.iter().take(4).enumerate() {
        b[5 + 3 * i..8 + 3 * i].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

struct FourZone;

impl Driver for FourZone {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&four_zone_report(colors))
    }
}

fn open_four_zone(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("lenovo"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        4,
    );
    Ok(Hid::boxed("lenovo 4 zone", info.open(api)?, FourZone, dev))
}

/// (byte, bit) of each K500 key in the custom mask, in LED order.
#[rustfmt::skip]
const K500_KEYS: [(u8, u8); 104] = [
    (0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (0, 7), (1, 0), (1, 1), (1, 2), (1, 3), (1, 4),
    (1, 5), (1, 6), (1, 7), (2, 0), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (2, 6), (2, 7), (3, 0), (3, 1),
    (3, 2), (3, 3), (3, 4), (3, 5), (3, 6), (3, 7), (7, 6), (12, 0), (12, 5), (13, 1), (14, 0), (4, 0), (4, 1),
    (4, 2), (4, 3), (4, 4), (4, 5), (4, 6), (4, 7), (5, 0), (5, 1), (5, 2), (5, 3), (5, 4), (5, 5), (5, 6),
    (5, 7), (7, 7), (12, 1), (12, 6), (13, 2), (14, 1), (6, 0), (6, 1), (6, 2), (6, 3), (6, 4), (6, 5), (6, 6),
    (6, 7), (7, 0), (7, 1), (7, 2), (7, 3), (7, 4), (12, 2), (12, 7), (13, 3), (8, 0), (8, 1), (8, 2), (8, 3),
    (8, 4), (8, 5), (8, 6), (8, 7), (9, 0), (9, 1), (9, 2), (9, 3), (9, 6), (12, 3), (13, 0), (13, 4), (14, 3),
    (10, 0), (10, 1), (10, 2), (10, 5), (11, 0), (11, 1), (11, 2), (11, 3), (11, 6), (11, 7), (9, 7), (12, 4),
    (13, 5),
];

#[rustfmt::skip]
const K500_MAP: [u32; 6 * 23] = [
    0, NA, 1, 2, 3, 4, NA, 5, 6, 7, 8, NA, 9, 10, 11, 12, NA, 13, 14, 15, NA, NA, NA,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, NA, 30, 31, 32, NA, 33, 34, 35, 36,
    37, NA, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, NA, 54, 55, 56, 57,
    58, NA, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, NA, NA, NA, NA, NA, 71, 72, 73, NA,
    74, NA, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, NA, NA, NA, 86, NA, NA, 87, 88, 89, 90,
    91, 92, 93, NA, NA, NA, 94, NA, NA, NA, NA, 95, 96, 97, 98, 99, 100, 101, NA, 102, NA, 103, NA,
];

#[rustfmt::skip]
const K500_PALETTE: [u8; 30] = [
    0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00,
    0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x00,
];

/// The colour DeviceUpdateLEDs picks for Custom: the first lit key that changed, else the first lit key.
pub fn k500_custom_color(colors: &[Color], last: &[Color]) -> Color {
    let lit = |c: &&Color| **c != Color::BLACK;
    colors
        .iter()
        .take(104)
        .enumerate()
        .find(|(i, c)| lit(c) && last.get(*i) != Some(c))
        .map(|(_, c)| *c)
        .or_else(|| colors.iter().take(104).find(lit).copied())
        .unwrap_or(Color::BLACK)
}

/// LenovoK500Controller::SetMode(Customize, colours, brightness 2, speed 3): a feature report then the data.
pub fn k500_reports(colors: &[Color], custom: Color) -> ([u8; 9], [u8; 65]) {
    let feature = [0x00, 0x14, 0x00, 0x5A, 0, 0, 0, 0, 0];
    let mut b = [0u8; 65];
    b[1..5].copy_from_slice(&[0x01, 0x06, 0x02, 0x03]);
    b[0x11..0x11 + 30].copy_from_slice(&K500_PALETTE);
    for (i, c) in colors.iter().take(104).enumerate() {
        if *c != Color::BLACK {
            let (byte, bit) = K500_KEYS[i];
            b[0x31 + byte as usize] |= 1 << bit;
        }
    }
    b[0x11..0x14].copy_from_slice(&[custom.0, custom.1, custom.2]);
    (feature, b)
}

#[derive(Default)]
struct K500 {
    last: Vec<Color>,
}

impl Driver for K500 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let custom = k500_custom_color(colors, &self.last);
        self.last = colors
            .iter()
            .map(|c| if *c != Color::BLACK { custom } else { *c })
            .collect();
        let (f, d) = k500_reports(colors, custom);
        io.send_feature(&f)?;
        io.write(&d)
    }
}

fn open_k500(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = matrix(
        info.device_id("lenovo"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        (6, 23),
        &K500_MAP,
        104,
    );
    Ok(Hid::boxed(
        "lenovo legion k500",
        info.open(api)?,
        K500::default(),
        dev,
    ))
}

pub const Y740: u16 = 0xC935;
pub const Y750: u16 = 0xC956;
pub const Y750S: u16 = 0xC957;
pub const Y760: u16 = 0xC968;
pub const Y760S: u16 = 0xC967;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kb {
    Iso,
    Ansi,
    Japan,
}

/// sendBasicInstruction / getInformation request.
pub fn basic(instruction: u8) -> [u8; 192] {
    let mut b = [0u8; 192];
    b[0] = 0x07;
    b[1] = instruction;
    b
}

fn byte4(r: &[u8]) -> Option<u8> {
    r.get(4).copied().filter(|_| r.len() > 4)
}

/// The keyboard type RGBController_LenovoUSB reads from the getInformation answers (info 0x01, 0x04, 0x07).
pub fn usb_keyboard(pid: u16, info1: &[u8], info4: &[u8], info7: &[u8]) -> (Kb, bool) {
    let fifteen = pid == Y740 && byte4(info1).is_some_and(|b| b <= 100);
    let kb = match (pid, byte4(info1), byte4(info4), byte4(info7)) {
        (Y740, _, Some(16..=48), _) => Kb::Iso,
        (Y750, _, Some(41), _) | (Y760, _, _, Some(41)) => Kb::Japan,
        (Y750, _, Some(16..=40), _) | (Y760, _, _, Some(16..=40)) => Kb::Iso,
        (Y750S | Y760S, Some(0x97), _, _) => Kb::Japan,
        (Y750S, Some(0x91), _, _) | (Y760S, Some(0x8F), _, _) => Kb::Iso,
        _ => Kb::Ansi,
    };
    (kb, fifteen)
}

/// RGBController_LenovoUSB::SetupZones.
pub fn usb_zones(pid: u16, kb: Kb, fifteen: bool) -> Vec<&'static Zone> {
    let y760_kbd = match kb {
        Kb::Japan => &LENOVO_LEGION_Y760_KBD_JP,
        Kb::Iso => &LENOVO_LEGION_Y760_KBD_ISO,
        Kb::Ansi => &LENOVO_LEGION_Y760_KBD_ANSI,
    };
    match pid {
        Y740 => {
            let kbd = match (fifteen, kb) {
                (true, Kb::Iso) => &LENOVO_LEGION_Y740_15_KBD_ISO,
                (true, _) => &LENOVO_LEGION_Y740_15_KBD_ANSI,
                (false, Kb::Iso) => &LENOVO_LEGION_Y740_17_KBD_ISO,
                (false, _) => &LENOVO_LEGION_Y740_17_KBD_ANSI,
            };
            vec![
                kbd,
                &LENOVO_LEGION_Y740_LOGO,
                &LENOVO_LEGION_Y740_PWRBTN,
                &LENOVO_LEGION_Y740_VENTS,
                &LENOVO_LEGION_Y740_PORTS,
            ]
        }
        Y750 => vec![
            y760_kbd,
            &LENOVO_LEGION_Y750_LOGO,
            &LENOVO_LEGION_Y750_VENTS,
            &LENOVO_LEGION_Y750_NEON,
        ],
        Y760 => vec![
            y760_kbd,
            &LENOVO_LEGION_Y760_LOGO,
            &LENOVO_LEGION_Y760_VENT_LEFT,
            &LENOVO_LEGION_Y760_VENT_RIGHT,
            &LENOVO_LEGION_Y760_VENT_BACK_RIGHT,
            &LENOVO_LEGION_Y760_VENT_BACK_LEFT,
            &LENOVO_LEGION_Y760_NEON,
        ],
        _ => vec![y760_kbd],
    }
}

/// (zone id << 8) + led_num of every LED, in order.
pub fn led_values(zones: &[&Zone]) -> Vec<u16> {
    zones
        .iter()
        .flat_map(|z| z.leds.iter().map(move |&n| (z.id as u16) << 8 | n as u16))
        .collect()
}

/// setZoneLeds: up to 0x2F LEDs per 192-byte feature report.
pub fn zone_reports(zone: u8, leds: &[(u8, Color)]) -> Vec<[u8; 192]> {
    leds.chunks(0x2F)
        .map(|chunk| {
            let mut b = [0u8; 192];
            b[..3].copy_from_slice(&[0x07, 0xA0u8.wrapping_add(zone), chunk.len() as u8]);
            for (k, (n, c)) in chunk.iter().enumerate() {
                b[4 + 4 * k..8 + 4 * k].copy_from_slice(&[*n, c.0, c.1, c.2]);
            }
            b
        })
        .collect()
}

/// RGBController_LenovoUSB::DeviceUpdateLEDs, which batches consecutive LEDs of one zone.
pub fn usb_frame(values: &[u16], colors: &[Color]) -> Vec<[u8; 192]> {
    let mut out = Vec::new();
    let mut prev = 0u8;
    let mut batch: Vec<(u8, Color)> = Vec::new();
    for (v, c) in values.iter().zip(colors) {
        let zone = (v >> 8) as u8;
        if zone != prev && prev != 0 {
            out.extend(zone_reports(prev, &batch));
            batch.clear();
        }
        prev = zone;
        batch.push((*v as u8, *c));
    }
    if !batch.is_empty() {
        out.extend(zone_reports(prev, &batch));
    }
    out
}

struct LenovoUsb {
    values: Vec<u16>,
}

impl Driver for LenovoUsb {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&basic(0xB2))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        usb_frame(&self.values, colors)
            .iter()
            .try_for_each(|r| io.send_feature(r))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.send_feature(&basic(0xB1));
    }
}

fn information(io: &mut dyn Io, id: u8) -> Vec<u8> {
    if io.send_feature(&basic(id)).is_err() {
        return Vec::new();
    }
    let mut r = [0u8; 192];
    r[0] = 0x07;
    match io.get_feature(&mut r) {
        Ok(n) => r[..n.min(192)].to_vec(),
        Err(_) => Vec::new(),
    }
}

fn open_usb(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut hid = info.open(api)?;
    let pid = info.pid;
    let mut ask = |id: u8, wanted: bool| {
        if wanted {
            information(&mut hid, id)
        } else {
            Vec::new()
        }
    };
    let i1 = ask(0x01, matches!(pid, Y740 | Y750S | Y760S));
    let i4 = ask(0x04, matches!(pid, Y740 | Y750));
    let i7 = ask(0x07, pid == Y760);
    let (kb, fifteen) = usb_keyboard(pid, &i1, &i4, &i7);
    let zones = usb_zones(pid, kb, fifteen);
    let values = led_values(&zones);
    let kbd = zones[0];
    let dev = matrix(
        info.device_id("lenovo"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        (kbd.height, kbd.width),
        kbd.map.unwrap_or(&[]),
        values.len(),
    );
    Ok(Hid::boxed("lenovo legion", hid, LenovoUsb { values }, dev))
}

macro_rules! pu {
    ($name:expr, $vid:expr, $pid:expr, $page:expr, $usage:expr, $open:expr) => {
        Detector {
            name: $name,
            vid: $vid,
            pid: $pid,
            interface: None,
            usage_page: Some($page),
            usage: Some($usage),
            open: $open,
        }
    };
}

pub static DETECTORS: [Detector; 15] = [
    pu!("Lenovo Legion Y740", ITE, Y740, 0xFF89, 0x07, open_usb),
    pu!("Lenovo Legion 7 Gen 5", ITE, Y750, 0xFF89, 0x07, open_usb),
    pu!("Lenovo Legion 7S Gen 5", ITE, Y750S, 0xFF89, 0x07, open_usb),
    pu!("Lenovo Legion 7 Gen 6", ITE, Y760, 0xFF89, 0x07, open_usb),
    pu!("Lenovo Legion 7S Gen 6", ITE, Y760S, 0xFF89, 0x07, open_usb),
    pu!(
        "Lenovo Ideapad 3-15ach6",
        ITE,
        0xC963,
        0xFF89,
        0xCC,
        open_four_zone
    ),
    pu!("Lenovo 5 2023", ITE, 0xC985, 0xFF89, 0xCC, open_four_zone),
    pu!(
        "Lenovo 5 2023 Ideapad",
        ITE,
        0xC984,
        0xFF89,
        0xCC,
        open_four_zone
    ),
    pu!("Lenovo 5 2022", ITE, 0xC975, 0xFF89, 0xCC, open_four_zone),
    pu!(
        "Lenovo 5 2022 Ideapad",
        ITE,
        0xC973,
        0xFF89,
        0xCC,
        open_four_zone
    ),
    pu!("Lenovo 5 2021", ITE, 0xC965, 0xFF89, 0xCC, open_four_zone),
    pu!(
        "Lenovo 5 2021 Ideapad",
        ITE,
        0xC963,
        0xFF89,
        0xCC,
        open_four_zone
    ),
    pu!("Lenovo 5 2020", ITE, 0xC955, 0xFF89, 0xCC, open_four_zone),
    Detector {
        name: "Lenovo Legion K500",
        vid: LENOVO,
        pid: 0x60D5,
        interface: Some(3),
        usage_page: None,
        usage: None,
        open: open_k500,
    },
    pu!("Lenovo", LENOVO, 0xC955, 0xFF89, 0xCC, open_board),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Mock, Op, pad};
    use super::*;

    const C1: Color = Color(0x11, 0x22, 0x33);
    const C2: Color = Color(0x44, 0x55, 0x66);

    // RGBController_LenovoMotherboard::DeviceUpdateLEDs -> LenovoMotherboardController::SetMode per LED.
    #[test]
    fn board_frame() {
        let [a, b] = board_reports(0x12, C1);
        assert_eq!(
            a.to_vec(),
            pad(&[0xCC, 0x12, 0x01, 0x01, 0x04, 0x11, 0x22, 0x33], 64)
        );
        let mut want = pad(&[0xCC, 0x28, 0x06], 64);
        want[33..40].copy_from_slice(&[0x12, 0x01, 0x01, 0x04, 0x11, 0x22, 0x33]);
        assert_eq!(b.to_vec(), want);
        let m = Mock::default();
        let dev = line("b".into(), "B", DeviceKind::Motherboard, BOARD, 2);
        let mut be = Hid::new("t", Box::new(m.clone()), Board, dev);
        be.push("b", &[C1, C2]).unwrap();
        let ops = m.take();
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[2], Op::Feature(board_reports(0x11, C2)[0].to_vec()));
    }

    // Lenovo4ZoneUSBController::setMode with the KeyboardState Direct uses.
    #[test]
    fn four_zone_frame() {
        let r = four_zone_report(&[C1, C2, Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(
            r.to_vec(),
            pad(
                &[
                    0xCC, 0x16, 0x01, 0x01, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 1, 2, 3, 4,
                    5, 6, 0x00, 0x00, 0x00
                ],
                33
            )
        );
    }

    // RGBController_LenovoK500::DeviceUpdateLEDs (Custom) -> LenovoK500Controller::SetMode.
    #[test]
    fn k500_custom_frame() {
        let mut colors = vec![Color::BLACK; 104];
        colors[0] = C1;
        colors[103] = C2;
        let custom = k500_custom_color(&colors, &[]);
        assert_eq!(custom, C1);
        let (f, d) = k500_reports(&colors, custom);
        assert_eq!(f, [0x00, 0x14, 0x00, 0x5A, 0, 0, 0, 0, 0]);
        assert_eq!(d[..5], [0x00, 0x01, 0x06, 0x02, 0x03]);
        assert_eq!(d[0x11..0x14], [0x11, 0x22, 0x33]);
        assert_eq!(d[0x14..0x2F], K500_PALETTE[3..]);
        assert_eq!(d[0x31], 0x01);
        // NumDecimal is byte 13 bit 5
        assert_eq!(d[0x31 + 13], 0x20);
        assert_eq!(d[0x2F..0x31], [0, 0]);
        // a newly lit key wins over unchanged ones
        let mut next = colors.clone();
        next[0] = C1;
        next[103] = C2;
        let last: Vec<Color> = colors
            .iter()
            .map(|c| if *c != Color::BLACK { C1 } else { *c })
            .collect();
        assert_eq!(k500_custom_color(&next, &last), C2);
    }

    // RGBController_LenovoUSB constructor's keyboard detection.
    #[test]
    fn usb_keyboard_detection() {
        let r = |b: u8| vec![0x07, 0, 0, 0, b];
        assert_eq!(usb_keyboard(Y740, &r(90), &r(20), &[]), (Kb::Iso, true));
        assert_eq!(usb_keyboard(Y740, &r(120), &r(50), &[]), (Kb::Ansi, false));
        assert_eq!(usb_keyboard(Y750, &[], &r(41), &[]).0, Kb::Japan);
        assert_eq!(usb_keyboard(Y750S, &r(0x91), &[], &[]).0, Kb::Iso);
        assert_eq!(usb_keyboard(Y760S, &r(0x91), &[], &[]).0, Kb::Ansi);
        assert_eq!(usb_keyboard(Y760, &[], &[], &r(30)).0, Kb::Iso);
        assert_eq!(usb_zones(Y760, Kb::Ansi, false).len(), 7);
        assert_eq!(led_values(&usb_zones(Y760S, Kb::Ansi, false)).len(), 145);
    }

    // LenovoUSBController::setZoneLeds splits at 0x2F LEDs.
    #[test]
    fn zone_reports_split() {
        let leds: Vec<(u8, Color)> = (0..50).map(|i| (i as u8 + 1, C1)).collect();
        let r = zone_reports(1, &leds);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0][..8], [0x07, 0xA1, 0x2F, 0x00, 0x01, 0x11, 0x22, 0x33]);
        assert_eq!(r[0][4 + 4 * 46..8 + 4 * 46], [47, 0x11, 0x22, 0x33]);
        assert_eq!(r[1][..8], [0x07, 0xA1, 0x03, 0x00, 48, 0x11, 0x22, 0x33]);
    }

    // RGBController_LenovoUSB::DeviceUpdateLEDs batching, start (0xB2) and shutdown (0xB1).
    #[test]
    fn usb_frame_batches_by_zone() {
        let values = [0x0101, 0x0102, 0x0205, 0x0301];
        let f = usb_frame(&values, &[C1, C1, C2, C2]);
        assert_eq!(f.len(), 3);
        assert_eq!(
            f[0][..12],
            [
                0x07, 0xA1, 2, 0, 0x01, 0x11, 0x22, 0x33, 0x02, 0x11, 0x22, 0x33
            ]
        );
        assert_eq!(f[1][..8], [0x07, 0xA2, 1, 0, 0x05, 0x44, 0x55, 0x66]);
        assert_eq!(f[2][..3], [0x07, 0xA3, 1]);
        let m = Mock::default();
        let dev = line("u".into(), "U", DeviceKind::Keyboard, KEYBOARD, 4);
        let mut b = Hid::new(
            "t",
            Box::new(m.clone()),
            LenovoUsb {
                values: values.to_vec(),
            },
            dev,
        );
        b.push("u", &[C1; 4]).unwrap();
        drop(b);
        let ops = m.take();
        assert_eq!(ops[0], Op::Feature(pad(&[0x07, 0xB2], 192)));
        assert_eq!(ops.last(), Some(&Op::Feature(pad(&[0x07, 0xB1], 192))));
    }
}
