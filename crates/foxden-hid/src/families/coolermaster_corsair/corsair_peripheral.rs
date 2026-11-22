// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, line, matrix, rows, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1B1C;
const I1: Option<i32> = Some(1);
const PAGE: Option<u16> = Some(0xFFC2);
const NA: u32 = u32::MAX;
const ISO: u8 = 0x01;
const DARK_CORE: u16 = 0x1B35;
const TKL_COLS: usize = 19;

pub static DETECTORS: &[Detector] = &[
    det!("Corsair K55 RGB", VID, 0x1B3D, I1, PAGE, None, open),
    det!("Corsair K65 RGB", VID, 0x1B17, I1, PAGE, None, open),
    det!("Corsair K65 LUX RGB", VID, 0x1B37, I1, PAGE, None, open),
    det!(
        "Corsair K65 RGB RAPIDFIRE",
        VID,
        0x1B39,
        I1,
        PAGE,
        None,
        open
    ),
    det!("Corsair K68 RGB", VID, 0x1B4F, I1, PAGE, None, open),
    det!("Corsair K68 RED", VID, 0x1B3F, I1, PAGE, None, open),
    det!("Corsair K68 RED SHADOW", VID, 0x1BA5, I1, PAGE, None, open),
    det!("Corsair K70 RGB", VID, 0x1B13, I1, PAGE, None, open),
    det!("Corsair K70 LUX", VID, 0x1B36, I1, PAGE, None, open),
    det!("Corsair K70 LUX RGB", VID, 0x1B33, I1, PAGE, None, open),
    det!(
        "Corsair K70 RGB RAPIDFIRE",
        VID,
        0x1B38,
        I1,
        PAGE,
        None,
        open
    ),
    det!(
        "Corsair K70 RAPIDFIRE Red",
        VID,
        0x1B3A,
        I1,
        PAGE,
        None,
        open
    ),
    det!("Corsair K70 RGB MK.2", VID, 0x1B49, I1, PAGE, None, open),
    det!("Corsair K70 RGB MK.2 SE", VID, 0x1B6B, I1, PAGE, None, open),
    det!(
        "Corsair K70 RGB MK.2 Low Profile",
        VID,
        0x1B55,
        I1,
        PAGE,
        None,
        open
    ),
    det!(
        "Corsair K70 RGB PRO Optomechanical",
        VID,
        0x1BC6,
        I1,
        PAGE,
        None,
        open
    ),
    det!("Corsair K95 RGB", VID, 0x1B11, I1, PAGE, None, open),
    det!(
        "Corsair K95 RGB PLATINUM",
        VID,
        0x1B2D,
        I1,
        PAGE,
        None,
        open
    ),
    det!(
        "Corsair K95 RGB PLATINUM SE",
        VID,
        0x1B82,
        I1,
        PAGE,
        None,
        open
    ),
    det!("Corsair Strafe", VID, 0x1B20, I1, PAGE, None, open),
    det!("Corsair Strafe", VID, 0x1B15, I1, PAGE, None, open),
    det!("Corsair Strafe Red", VID, 0x1B44, I1, PAGE, None, open),
    det!("Corsair Strafe MK.2", VID, 0x1B48, I1, PAGE, None, open),
    det!("Corsair Glaive RGB", VID, 0x1B34, I1, PAGE, None, open),
    det!("Corsair Glaive RGB PRO", VID, 0x1B74, I1, PAGE, None, open),
    det!("Corsair Harpoon RGB", VID, 0x1B3C, I1, PAGE, None, open),
    det!("Corsair Harpoon RGB PRO", VID, 0x1B75, I1, PAGE, None, open),
    det!("Corsair Ironclaw RGB", VID, 0x1B5D, I1, PAGE, None, open),
    det!("Corsair M65", VID, 0x1B12, I1, PAGE, None, open),
    det!("Corsair M65 PRO", VID, 0x1B2E, I1, PAGE, None, open),
    det!("Corsair M65 RGB Elite", VID, 0x1B5A, I1, PAGE, None, open),
    det!("Corsair Nightsword", VID, 0x1B5C, I1, PAGE, None, open),
    det!("Corsair Scimitar RGB", VID, 0x1B1E, I1, PAGE, None, open),
    det!(
        "Corsair Scimitar PRO RGB",
        VID,
        0x1B3E,
        I1,
        PAGE,
        None,
        open
    ),
    det!(
        "Corsair Scimitar Elite RGB",
        VID,
        0x1B8B,
        I1,
        PAGE,
        None,
        open
    ),
    det!("Corsair Sabre RGB", VID, 0x1B2F, I1, PAGE, None, open),
    det!("Corsair Sabre RGB", VID, 0x1B14, I1, PAGE, None, open),
    det!(
        "Corsair Sabre RGB Optical",
        VID,
        0x1B32,
        I1,
        PAGE,
        None,
        open
    ),
    det!("Corsair Sabre RGB Laser", VID, 0x1B19, I1, PAGE, None, open),
    det!("Corsair Katar RGB", VID, 0x1B22, I1, PAGE, None, open),
    det!("Corsair Dark Core RGB", VID, 0x1B35, I1, PAGE, None, open),
    det!("Corsair K63", VID, 0x1B40, I1, PAGE, None, open),
    det!(
        "Corsair MM800 RGB Polaris",
        VID,
        0x1B3B,
        None,
        PAGE,
        None,
        open
    ),
    det!("Corsair ST100 RGB", VID, 0x0A34, None, PAGE, None, open),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Logical {
    Normal,
    K95Plat,
    K95,
    K55,
    K70Mk2,
    K68,
    K70LuxRgb,
    /// Tenkeyless, red LEDs only.
    K63,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Logical layout and physical layout (ANSI 0, ISO 1, ...).
    Keyboard(Logical, u8),
    Mouse,
    /// Three zones set through static-animation packets.
    DarkCore,
    Mousemat,
    Stand,
}

pub type Report = [u8; 65];

fn packet(bytes: &[u8]) -> Report {
    let mut r = [0u8; 65];
    r[..bytes.len()].copy_from_slice(bytes);
    r
}

/// Read Firmware Info request.
pub fn firmware_query() -> Report {
    packet(&[0x00, 0x0E, 0x01])
}

/// The device type from a firmware info reply; `offset` is 1 when it came as a feature report.
pub fn parse_firmware(buf: &[u8; 65], offset: usize) -> Option<Kind> {
    match buf[0x14 + offset] {
        0xC0 => {
            let pid = ((buf[0x0E] as u16) << 8) + buf[0x0F] as u16;
            let logical = match pid {
                0x1B2D => Logical::K95Plat,
                0x1B11 => Logical::K95,
                0x1B3D => Logical::K55,
                0x1B33 => Logical::K70LuxRgb,
                0x1B38 | 0x1B49 | 0x1B6B | 0x1B55 | 0x1B48 => Logical::K70Mk2,
                0x1B4F => Logical::K68,
                0x1B40 => Logical::K63,
                _ => Logical::Normal,
            };
            let physical = match buf[0x17 + offset] {
                p @ 0..=4 => p,
                _ => 0,
            };
            Some(Kind::Keyboard(logical, physical))
        }
        0xC1 => Some(Kind::Mouse),
        0xC2 => {
            let pid = ((buf[0x0F] as u16) << 8) + buf[0x0E] as u16;
            Some(if pid == 0x0A34 {
                Kind::Stand
            } else {
                Kind::Mousemat
            })
        }
        _ => None,
    }
}

fn needs_special(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Keyboard(
            Logical::K55 | Logical::K95Plat | Logical::K70Mk2 | Logical::K68,
            _
        )
    )
}

/// SpecialFunctionControl
pub fn special_function() -> Report {
    packet(&[0x00, 0x07, 0x04, 0x02])
}

/// LightingControl: software control, with a per-type byte 5.
pub fn lighting_control(kind: Kind) -> Report {
    let b5 = match kind {
        Kind::Keyboard(..) | Kind::Stand => 0x03,
        Kind::Mouse | Kind::DarkCore => 0x01,
        Kind::Mousemat => 0x04,
    };
    packet(&[0x00, 0x07, 0x05, 0x02, 0x00, b5])
}

/// SetupK55AndK95LightingControl: one control packet and four of 30 key ids each.
pub fn k55_k95_setup(logical: Logical, physical: u8) -> Vec<Report> {
    let skipped = match (physical, logical) {
        (ISO, Logical::K70Mk2 | Logical::K70LuxRgb) => KEY_MAPPING_K70_MK2_PLAT_ISO,
        (ISO, _) => KEY_MAPPING_K95_PLAT_ISO,
        _ => KEY_MAPPING_K95_PLAT_ANSI,
    };
    let mut out = vec![packet(&[0x00, 0x07, 0x05, 0x08, 0x00, 0x01])];
    let mut id: u32 = 0;
    for _ in 0..4 {
        let mut r = packet(&[0x00, 0x07, 0x40, 0x1E]);
        for j in 0..30 {
            for &s in skipped {
                if id == s as u32 {
                    id += 1;
                }
            }
            r[5 + 2 * j] = id as u8;
            r[6 + 2 * j] = 0xC0;
            id += 1;
        }
        out.push(r);
    }
    out
}

/// What the constructor (and ReadFirmwareInfo for mice and pads) sends to take software control.
pub fn init(kind: Kind) -> Vec<Report> {
    let mut out = Vec::new();
    if !matches!(kind, Kind::Keyboard(..))
        || needs_special(kind)
        || matches!(kind, Kind::Keyboard(Logical::K63, _))
    {
        out.push(special_function());
    }
    out.push(lighting_control(kind));
    if let Kind::Keyboard(logical, physical) = kind
        && needs_special(kind)
    {
        out.extend(k55_k95_setup(logical, physical));
    }
    out
}

fn stream(id: u8, data: &[u8]) -> Report {
    let mut r = packet(&[0x00, 0x7F, id, data.len() as u8]);
    r[5..5 + data.len()].copy_from_slice(data);
    r
}

fn submit_full(channel: u8, finish: u8) -> Report {
    packet(&[0x00, 0x07, 0x28, channel, 0x03, finish])
}

/// SetLEDsKeyboardFull: three channels, each three stream packets and a submit.
pub fn keyboard_full(logical: Logical, colors: &[Color]) -> Vec<Report> {
    let (keys, last) = match logical {
        Logical::K95Plat => (KEYS_K95_PLAT, 48),
        Logical::K95 => (KEYS_K95, 48),
        Logical::K70Mk2 | Logical::K70LuxRgb => (KEYS_K70_MK2, 24),
        _ => (KEYS, 24),
    };
    let mut planes = [[0u8; 168]; 3];
    for (c, &k) in colors.iter().zip(keys) {
        planes[0][k as usize] = c.0;
        planes[1][k as usize] = c.1;
        planes[2][k as usize] = c.2;
    }
    let mut out = Vec::new();
    for (ch, plane) in planes.iter().enumerate() {
        out.push(stream(1, &plane[..60]));
        out.push(stream(2, &plane[60..120]));
        out.push(stream(3, &plane[120..120 + last]));
        out.push(submit_full(ch as u8 + 1, if ch == 2 { 2 } else { 1 }));
    }
    out
}

/// LED indices of the full-size map left of the numpad, in order.
fn tkl_leds() -> Vec<u32> {
    let mut v: Vec<u32> = MATRIX_MAP
        .iter()
        .flat_map(|row| row[..TKL_COLS].iter().copied())
        .filter(|&i| i != NA)
        .collect();
    v.sort_unstable();
    v
}

/// Red channel only: three bulk packets of one intensity per key, then one submit.
pub fn keyboard_mono(colors: &[Color]) -> Vec<Report> {
    let mut plane = [0u8; 168];
    for (c, i) in colors.iter().zip(tkl_leds()) {
        plane[KEYS[i as usize] as usize] = c.0.max(c.1).max(c.2);
    }
    vec![
        stream(1, &plane[..60]),
        stream(2, &plane[60..120]),
        stream(3, &plane[120..168]),
        submit_full(1, 1),
    ]
}

/// One static-colour packet per Dark Core zone: front, thumb, rear.
pub fn dark_core(colors: &[Color]) -> Vec<Report> {
    (0..3u8)
        .map(|z| {
            let c = colors.get(z as usize).copied().unwrap_or(Color::BLACK);
            let mut r = packet(&[0x00, 0x07, 0xAA, 0x00, 0x00, 1 << z, 0x07, 0x00, 0x00, 100]);
            r[10..13].copy_from_slice(&[c.0, c.1, c.2]);
            r[16] = 5 - z;
            r
        })
        .collect()
}

/// SubmitKeyboardZonesColors, for the K55's three zones.
pub fn keyboard_zones(left: Color, mid: Color, right: Color) -> Report {
    packet(&[
        0x00, 0x07, 0x25, 0x00, 0x00, left.0, left.1, left.2, mid.0, mid.1, mid.2, right.0,
        right.1, right.2,
    ])
}

/// SubmitMouseColors: zone index then RGB per zone.
pub fn mouse(colors: &[Color]) -> Report {
    let mut r = packet(&[0x00, 0x07, 0x22, colors.len() as u8, 0x00]);
    for (i, c) in colors.iter().enumerate() {
        r[i * 4 + 5..i * 4 + 9].copy_from_slice(&[i as u8, c.0, c.1, c.2]);
    }
    r
}

/// SubmitMousematColors: RGB per zone.
pub fn mousemat(colors: &[Color]) -> Report {
    let mut r = packet(&[0x00, 0x07, 0x22, colors.len() as u8, 0x00]);
    for (i, c) in colors.iter().enumerate() {
        r[i * 3 + 5..i * 3 + 8].copy_from_slice(&[c.0, c.1, c.2]);
    }
    r
}

/// SetLEDs for the ST100: the logo sits in the middle of the base strip.
pub fn stand(colors: &[Color]) -> Report {
    let mut remap = vec![Color::BLACK; colors.len()];
    for (i, &to) in ST100.iter().enumerate() {
        remap[to as usize] = colors[i];
    }
    mousemat(&remap)
}

pub fn frame(kind: Kind, colors: &[Color]) -> Vec<Report> {
    match kind {
        Kind::Keyboard(Logical::K55, _) => vec![keyboard_zones(colors[0], colors[1], colors[2])],
        Kind::Keyboard(Logical::K63, _) => keyboard_mono(colors),
        Kind::Keyboard(logical, _) => keyboard_full(logical, colors),
        Kind::Mouse => vec![mouse(colors)],
        Kind::DarkCore => dark_core(colors),
        Kind::Mousemat => vec![mousemat(colors)],
        Kind::Stand => vec![stand(colors)],
    }
}

fn layout(kind: Kind) -> (DeviceKind, Vec<[f32; 2]>, [u32; 2]) {
    fn m<const C: usize>(map: &[[u32; C]], count: usize) -> (DeviceKind, Vec<[f32; 2]>, [u32; 2]) {
        let rows: Vec<&[u32]> = map.iter().map(|r| r.as_slice()).collect();
        let (leds, grid) = matrix(&rows, count);
        (DeviceKind::Keyboard, leds, grid)
    }
    match kind {
        Kind::Keyboard(Logical::K95Plat, _) => m(&MATRIX_MAP_K95_PLATINUM, 139),
        Kind::Keyboard(Logical::K95, _) => m(&MATRIX_MAP_K95, 135),
        Kind::Keyboard(Logical::K70LuxRgb, _) => m(&MATRIX_MAP_K70_LUX_RGB, 116),
        Kind::Keyboard(Logical::K70Mk2, _) => m(&MATRIX_MAP_K70_MK2, 116),
        Kind::Keyboard(Logical::K55, _) => (DeviceKind::Keyboard, line(3), [3, 1]),
        Kind::Keyboard(Logical::K63, _) => {
            let leds = tkl_leds();
            let map: Vec<Vec<u32>> = MATRIX_MAP
                .iter()
                .map(|row| {
                    row[..TKL_COLS]
                        .iter()
                        .map(|v| leds.iter().position(|l| l == v).map_or(NA, |i| i as u32))
                        .collect()
                })
                .collect();
            let rows: Vec<&[u32]> = map.iter().map(|r| r.as_slice()).collect();
            let (pos, grid) = matrix(&rows, leds.len());
            (DeviceKind::Keyboard, pos, grid)
        }
        Kind::Keyboard(..) => m(&MATRIX_MAP, 113),
        Kind::Mouse => (DeviceKind::Mouse, line(15), [15, 1]),
        Kind::DarkCore => (DeviceKind::Mouse, line(3), [3, 1]),
        Kind::Mousemat => (DeviceKind::Other, line(15), [15, 1]),
        Kind::Stand => (DeviceKind::Other, rows(&[8, 1]), [8, 2]),
    }
}

struct Peripheral {
    hid: HidDevice,
    kind: Kind,
}

impl Driver for Peripheral {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            for r in init(self.kind) {
                write(&self.hid, &r)?;
            }
        }
        for r in frame(self.kind, colors) {
            write(&self.hid, &r)?;
        }
        Ok(())
    }
}

/// ReadFirmwareInfo: up to ten tries, falling back to feature reports when nothing is read.
fn read_kind(hid: &HidDevice) -> Result<Kind, String> {
    for _ in 0..10 {
        let q = firmware_query();
        write(hid, &q)?;
        let mut buf = [0u8; 65];
        let n = hid
            .read_timeout(&mut buf, 1000)
            .map_err(|e| e.to_string())?;
        let mut offset = 0;
        if n == 0 {
            hid.send_feature_report(&q).map_err(|e| e.to_string())?;
            buf = [0u8; 65];
            hid.get_feature_report(&mut buf)
                .map_err(|e| e.to_string())?;
            offset = 1;
        }
        if let Some(kind) = parse_firmware(&buf, offset) {
            return Ok(kind);
        }
    }
    Err("device type is unknown".into())
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let kind = match read_kind(&hid)? {
        Kind::Mouse if info.pid == DARK_CORE => Kind::DarkCore,
        k => k,
    };
    let (dk, leds, grid) = layout(kind);
    let dev = device(
        info.device_id("corsairperipheral"),
        det.name,
        dk,
        leds,
        grid,
    );
    Ok(Lights::boxed(
        "corsair peripheral",
        dev,
        None,
        Peripheral { hid, kind },
    ))
}

const KEYS: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12,
    0x14, 0x15, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x2A, 0x2B, 0x2C, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3C, 0x3D,
    0x3E, 0x3F, 0x40, 0x42, 0x43, 0x44, 0x45, 0x48, 73, 74, 75, 76, 78, 79, 80, 81, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 108, 109, 110, 111, 112,
    113, 115, 116, 117, 120, 121, 122, 123, 124, 126, 127, 128, 129, 132, 133, 134, 135, 136, 137,
    139, 140, 141, 0x10, 114,
];
const KEYS_K70_MK2: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12,
    0x14, 0x15, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x2A, 0x2B, 0x2C, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3C, 0x3D,
    0x3E, 0x3F, 0x40, 0x42, 0x43, 0x44, 0x45, 0x48, 73, 74, 75, 76, 78, 79, 80, 81, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 108, 109, 110, 111, 112,
    113, 115, 116, 117, 120, 121, 122, 123, 124, 126, 127, 128, 129, 132, 133, 134, 135, 136, 137,
    139, 140, 141, 16, 114, 47, 59, 125,
];
const KEYS_K95_PLAT: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12,
    0x14, 0x15, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x2A, 0x2B, 0x2C, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3C, 0x3D,
    0x3E, 0x3F, 0x40, 0x42, 0x43, 0x44, 0x45, 0x48, 73, 74, 75, 76, 78, 79, 80, 81, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 108, 109, 110, 111, 112,
    113, 115, 116, 117, 120, 121, 122, 123, 124, 126, 127, 128, 129, 132, 133, 134, 135, 136, 137,
    139, 140, 141, 0x10, 114, 0x0a, 0x16, 0x22, 0x2e, 0x3a, 0x46, 125, 144, 145, 146, 158, 160,
    147, 148, 149, 150, 151, 152, 153, 154, 155, 159, 162, 161, 156, 157,
];
const KEYS_K95: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12,
    0x14, 0x15, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x2A, 0x2B, 0x2C, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3C, 0x3D,
    0x3E, 0x3F, 0x40, 0x42, 0x43, 0x44, 0x45, 0x48, 73, 74, 75, 76, 78, 79, 80, 81, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 108, 109, 110, 111, 112,
    113, 115, 116, 117, 120, 121, 122, 123, 124, 126, 127, 128, 129, 132, 133, 134, 135, 136, 137,
    139, 140, 141, 0x10, 114, 0x0a, 0x16, 0x22, 0x2e, 0x3a, 0x46, 0x52, 0x5e, 0x6a, 0x76, 0x3b,
    0x47, 0x53, 0x5f, 0x6b, 0x77, 0x83, 0x8f, 0x0b, 0x17, 0x23, 0x2f,
];
const ST100: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x05, 0x06, 0x07, 0x08, 0x04];
const KEY_MAPPING_K95_PLAT_ANSI: &[u8] = &[
    0x31, 0x3f, 0x41, 0x42, 0x51, 0x53, 0x55, 0x6f, 0x7e, 0x7f, 0x80, 0x81,
];
const KEY_MAPPING_K95_PLAT_ISO: &[u8] = &[
    0x3f, 0x41, 0x42, 0x50, 0x53, 0x55, 0x6f, 0x78, 0x7e, 0x7f, 0x80, 0x81,
];
const KEY_MAPPING_K70_MK2_PLAT_ISO: &[u8] = &[
    0x3f, 0x41, 0x42, 0x50, 0x53, 0x55, 0x6f, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x80,
    0x81,
];
const MATRIX_MAP: [[u32; 23]; 6] = [
    [
        0, NA, 10, 18, 28, 36, NA, 46, 55, 64, 74, NA, 84, 93, 102, 6, 15, 24, 33, 26, 35, 44, 53,
    ],
    [
        1, 11, 19, 29, 37, 47, 56, 65, 75, 85, 94, NA, 103, 7, 25, NA, 42, 51, 60, 62, 72, 82, 91,
    ],
    [
        2, NA, 12, 20, 30, 38, NA, 48, 57, 66, 76, 86, 95, 104, 70, 80, 34, 43, 52, 9, 17, 27, 100,
    ],
    [
        3, NA, 13, 21, 31, 39, NA, 49, 58, 67, 77, 87, 96, 105, 98, 112, NA, NA, NA, 45, 54, 63, NA,
    ],
    [
        4, 111, 22, 32, 40, 50, NA, 59, NA, 68, 78, 88, 97, 106, 61, NA, NA, 81, NA, 73, 83, 92,
        109,
    ],
    [
        5, 14, 23, NA, NA, NA, NA, 41, NA, NA, NA, NA, 69, 79, 89, 71, 90, 99, 108, 101, NA, 110,
        NA,
    ],
];
const MATRIX_MAP_K70_LUX_RGB: [[u32; 23]; 7] = [
    [
        NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 107, 8, NA, NA, 16, NA, NA,
    ],
    [
        0, NA, 10, 18, 28, 36, NA, 46, 55, 64, 74, NA, 84, 93, 102, 6, 15, 24, 33, 26, 35, 44, 53,
    ],
    [
        1, 11, 19, 29, 37, 47, NA, 56, 65, 75, 85, 94, 103, 7, 25, NA, 42, 51, 60, 62, 72, 82, 91,
    ],
    [
        2, NA, 12, 20, 30, 38, 48, 57, 66, 76, 86, NA, 95, 104, 70, 80, 34, 43, 52, 9, 17, 27, 100,
    ],
    [
        3, NA, 13, 21, 31, 39, 49, 58, 67, 77, 87, NA, 96, 105, 98, 112, NA, NA, NA, 45, 54, 63, NA,
    ],
    [
        4, 111, NA, 22, 32, 40, 50, 59, 68, 78, 88, 97, NA, 106, 61, NA, NA, 81, NA, 73, 83, 92,
        109,
    ],
    [
        5, NA, 14, 23, NA, NA, NA, 41, NA, NA, NA, 69, NA, 79, 89, 71, 90, 99, 108, 101, NA, 110,
        NA,
    ],
];
const MATRIX_MAP_K70_MK2: [[u32; 23]; 7] = [
    [
        NA, NA, NA, 115, 107, 8, NA, NA, NA, NA, NA, 113, 114, NA, NA, NA, NA, NA, NA, 16, NA, NA,
        NA,
    ],
    [
        0, NA, 10, 18, 28, 36, NA, 46, 55, 64, 74, NA, 84, 93, 102, 6, 15, 24, 33, 26, 35, 44, 53,
    ],
    [
        1, 11, 19, 29, 37, 47, 56, 65, 75, 85, 94, NA, 103, 7, 25, NA, 42, 51, 60, 62, 72, 82, 91,
    ],
    [
        2, NA, 12, 20, 30, 38, NA, 48, 57, 66, 76, 86, 95, 104, 70, 80, 34, 43, 52, 9, 17, 27, 100,
    ],
    [
        3, NA, 13, 21, 31, 39, NA, 49, 58, 67, 77, 87, 96, 105, 98, 112, NA, NA, NA, 45, 54, 63, NA,
    ],
    [
        4, 111, 22, 32, 40, 50, NA, 59, NA, 68, 78, 88, 97, 106, 61, NA, NA, 81, NA, 73, 83, 92,
        109,
    ],
    [
        5, 14, 23, NA, NA, NA, NA, 41, NA, NA, NA, NA, 69, 79, 89, 71, 90, 99, 108, 101, NA, 110,
        NA,
    ],
];
const MATRIX_MAP_K95_PLATINUM: [[u32; 24]; 7] = [
    [
        NA, NA, NA, 119, 107, 8, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 16, NA,
        NA, NA,
    ],
    [
        113, 0, NA, 10, 18, 28, 36, NA, 46, 55, 64, 74, NA, 84, 93, 102, 6, 15, 24, 33, 26, 35, 44,
        53,
    ],
    [
        114, 1, 11, 19, 29, 37, 47, 56, 65, 75, 85, 94, NA, 103, 7, 25, NA, 42, 51, 60, 62, 72, 82,
        91,
    ],
    [
        115, 2, NA, 12, 20, 30, 38, NA, 48, 57, 66, 76, 86, 95, 104, 70, 80, 34, 43, 52, 9, 17, 27,
        100,
    ],
    [
        116, 3, NA, 13, 21, 31, 39, NA, 49, 58, 67, 77, 87, 96, 105, 98, 112, NA, NA, NA, 45, 54,
        63, NA,
    ],
    [
        117, 4, 111, 22, 32, 40, 50, NA, 59, NA, 68, 78, 88, 97, 106, 61, NA, NA, 81, NA, 73, 83,
        92, 109,
    ],
    [
        118, 5, 14, 23, NA, NA, NA, NA, 41, NA, NA, NA, NA, 69, 79, 89, 71, 90, 99, 108, 101, NA,
        110, NA,
    ],
];
const MATRIX_MAP_K95: [[u32; 26]; 7] = [
    [
        NA, NA, NA, 131, 132, 133, 134, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, 107, 8, NA,
        NA, 16, NA, NA,
    ],
    [
        113, 114, 115, 0, NA, 10, 18, 28, 36, NA, 46, 55, 64, 74, NA, 84, 93, 102, 6, 15, 24, 33,
        26, 35, 44, 53,
    ],
    [
        116, 117, 118, 1, 11, 19, 29, 37, 47, 56, 65, 75, 85, 94, NA, 103, 7, 25, NA, 42, 51, 60,
        62, 72, 82, 91,
    ],
    [
        119, 120, 121, 2, NA, 12, 20, 30, 38, NA, 48, 57, 66, 76, 86, 95, 104, 70, 80, 34, 43, 52,
        9, 17, 27, 100,
    ],
    [
        122, 123, 124, 3, NA, 13, 21, 31, 39, NA, 49, 58, 67, 77, 87, 96, 105, 98, 112, NA, NA, NA,
        45, 54, 63, NA,
    ],
    [
        125, 126, 127, 4, 111, 22, 32, 40, 50, NA, 59, NA, 68, 78, 88, 97, 106, 61, NA, NA, 81, NA,
        73, 83, 92, 109,
    ],
    [
        128, 129, 130, 5, 14, 23, NA, NA, NA, NA, 41, NA, NA, NA, NA, 69, 79, 89, 71, 90, 99, 108,
        101, NA, 110, NA,
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn c(i: u8) -> Color {
        Color(i, i.wrapping_add(1), i.wrapping_add(2))
    }

    // CorsairPeripheralController::ReadFirmwareInfo (request)
    #[test]
    fn firmware_query_bytes() {
        assert_eq!(firmware_query(), packet(&[0x00, 0x0E, 0x01]));
    }

    // CorsairPeripheralController::ReadFirmwareInfo (reply decoding)
    #[test]
    fn firmware_reply() {
        let mut b = [0u8; 65];
        b[0x0E] = 0x1B;
        b[0x0F] = 0x49;
        b[0x14] = 0xC0;
        b[0x17] = 0x01;
        assert_eq!(
            parse_firmware(&b, 0),
            Some(Kind::Keyboard(Logical::K70Mk2, 1))
        );
        b[0x17] = 0x09;
        assert_eq!(
            parse_firmware(&b, 0),
            Some(Kind::Keyboard(Logical::K70Mk2, 0))
        );
        let mut b = [0u8; 65];
        b[0x15] = 0xC2;
        b[0x0E] = 0x34;
        b[0x0F] = 0x0A;
        assert_eq!(parse_firmware(&b, 1), Some(Kind::Stand));
        b[0x0E] = 0x3B;
        b[0x0F] = 0x1B;
        assert_eq!(parse_firmware(&b, 1), Some(Kind::Mousemat));
        b[0x15] = 0xC1;
        assert_eq!(parse_firmware(&b, 1), Some(Kind::Mouse));
        assert_eq!(parse_firmware(&[0u8; 65], 0), None);
    }

    // CorsairPeripheralController::LightingControl and SpecialFunctionControl
    #[test]
    fn control_packets() {
        assert_eq!(special_function(), packet(&[0, 0x07, 0x04, 0x02]));
        let kb = Kind::Keyboard(Logical::Normal, 0);
        assert_eq!(
            lighting_control(kb),
            packet(&[0, 0x07, 0x05, 0x02, 0, 0x03])
        );
        assert_eq!(
            lighting_control(Kind::Mouse),
            packet(&[0, 0x07, 0x05, 0x02, 0, 0x01])
        );
        assert_eq!(
            lighting_control(Kind::Mousemat),
            packet(&[0, 0x07, 0x05, 0x02, 0, 0x04])
        );
        assert_eq!(
            lighting_control(Kind::Stand),
            packet(&[0, 0x07, 0x05, 0x02, 0, 0x03])
        );
    }

    // CorsairPeripheralController constructor: plain keyboards only get LightingControl, mice also SpecialFunctionControl.
    #[test]
    fn init_sequences() {
        let kb = Kind::Keyboard(Logical::Normal, 0);
        assert_eq!(init(kb), vec![lighting_control(kb)]);
        assert_eq!(
            init(Kind::Mouse),
            vec![special_function(), lighting_control(Kind::Mouse)]
        );
        let k70 = Kind::Keyboard(Logical::K70Mk2, 0);
        let seq = init(k70);
        assert_eq!(seq.len(), 7);
        assert_eq!(seq[0], special_function());
        assert_eq!(seq[1], lighting_control(k70));
        assert_eq!(seq[2], packet(&[0, 0x07, 0x05, 0x08, 0, 0x01]));
    }

    // CorsairPeripheralController::SetupK55AndK95LightingControl (ANSI skip list)
    #[test]
    fn key_id_packets_skip_ansi_ids() {
        let seq = k55_k95_setup(Logical::K95Plat, 0);
        let ids = |r: &Report| (0..30).map(|j| r[5 + 2 * j]).collect::<Vec<u8>>();
        assert_eq!(&seq[1][..5], &[0, 0x07, 0x40, 0x1E, 0]);
        assert_eq!(ids(&seq[1]), (0..30).collect::<Vec<u8>>());
        assert!((0..30).all(|j| seq[1][6 + 2 * j] == 0xC0));
        let mut second: Vec<u8> = (30..49).collect();
        second.extend(50..61);
        assert_eq!(ids(&seq[2]), second);
        assert_eq!(&ids(&seq[3])[..4], &[61, 62, 64, 67]);
        assert_eq!(&ids(&seq[3])[16..20], &[80, 82, 84, 86]);
    }

    // CorsairPeripheralController::SetupK55AndK95LightingControl (K70 MK.2 ISO list)
    #[test]
    fn key_id_packets_k70_iso() {
        let seq = k55_k95_setup(Logical::K70Mk2, ISO);
        let ids: Vec<u8> = (0..30).map(|j| seq[2][5 + 2 * j]).collect();
        assert_eq!(ids, (30..60).collect::<Vec<u8>>());
        let ids: Vec<u8> = (0..30).map(|j| seq[3][5 + 2 * j]).collect();
        assert_eq!(&ids[..4], &[60, 61, 62, 64]);
    }

    // CorsairPeripheralController::SetLEDsKeyboardFull and SubmitKeyboardFullColors
    #[test]
    fn full_keyboard_frame() {
        let mut colors = vec![Color::BLACK; 113];
        colors[0] = Color(10, 20, 30);
        colors[112] = Color(1, 2, 3);
        let f = keyboard_full(Logical::Normal, &colors);
        assert_eq!(f.len(), 12);
        assert_eq!(&f[0][..6], &[0, 0x7F, 1, 60, 0, 10]);
        assert_eq!(&f[1][..4], &[0, 0x7F, 2, 60]);
        assert_eq!(f[1][5 + 54], 1);
        assert_eq!(&f[2][..4], &[0, 0x7F, 3, 24]);
        assert_eq!(f[3], packet(&[0, 0x07, 0x28, 1, 3, 1]));
        assert_eq!(f[4][5], 20);
        assert_eq!(f[5][59], 2);
        assert_eq!(f[7], packet(&[0, 0x07, 0x28, 2, 3, 1]));
        assert_eq!(f[8][5], 30);
        assert_eq!(f[9][59], 3);
        assert_eq!(f[11], packet(&[0, 0x07, 0x28, 3, 3, 2]));
        let k95 = keyboard_full(Logical::K95Plat, &vec![Color::BLACK; 139]);
        assert_eq!(k95[2][3], 48);
    }

    // CorsairPeripheralController::SetLEDsKeyboardFull uses the K70 MK.2 key table for the K70 LUX RGB.
    #[test]
    fn k70_key_table() {
        let mut colors = vec![Color::BLACK; 116];
        colors[115] = Color(9, 0, 0);
        let f = keyboard_full(Logical::K70LuxRgb, &colors);
        assert_eq!(f[2][5 + (125 - 120)], 9);
    }

    // CorsairPeripheralController::SubmitKeyboardZonesColors
    #[test]
    fn k55_zones() {
        assert_eq!(
            keyboard_zones(c(1), c(4), c(7)),
            packet(&[0, 0x07, 0x25, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
        );
    }

    // CorsairPeripheralController::SubmitMouseColors
    #[test]
    fn mouse_frame() {
        let mut colors = vec![Color::BLACK; 15];
        colors[0] = c(1);
        colors[14] = c(7);
        let r = mouse(&colors);
        assert_eq!(&r[..9], &[0, 0x07, 0x22, 15, 0, 0, 1, 2, 3]);
        assert_eq!(&r[9..13], &[1, 0, 0, 0]);
        assert_eq!(&r[61..65], &[14, 7, 8, 9]);
    }

    // CorsairPeripheralController::SubmitMousematColors
    #[test]
    fn mousemat_frame() {
        let colors: Vec<Color> = (0..15).map(|i| c(i * 3)).collect();
        let r = mousemat(&colors);
        assert_eq!(&r[..5], &[0, 0x07, 0x22, 15, 0]);
        assert_eq!(&r[5..50], &(0..45).collect::<Vec<u8>>()[..]);
        assert!(r[50..].iter().all(|&b| b == 0));
    }

    // CorsairPeripheralController::SetLEDs, DEVICE_TYPE_HEADSET_STAND remap
    #[test]
    fn stand_moves_logo() {
        let colors: Vec<Color> = (0..9).map(|i| Color(i, 0, 0)).collect();
        let r = stand(&colors);
        let reds: Vec<u8> = (0..9).map(|i| r[5 + i * 3]).collect();
        assert_eq!(reds, vec![0, 1, 2, 3, 8, 4, 5, 6, 7]);
        assert_eq!(r[3], 9);
    }

    #[test]
    fn k63_from_firmware() {
        let mut b = [0u8; 65];
        b[0x0E] = 0x1B;
        b[0x0F] = 0x40;
        b[0x14] = 0xC0;
        assert_eq!(parse_firmware(&b, 0), Some(Kind::Keyboard(Logical::K63, 0)));
        let k63 = Kind::Keyboard(Logical::K63, 0);
        assert_eq!(init(k63), vec![special_function(), lighting_control(k63)]);
    }

    // Red plane in three bulk packets of 60, 60 and 48, then submit colour 1 with finish 1.
    #[test]
    fn k63_mono_frame() {
        let k = Kind::Keyboard(Logical::K63, 0);
        let n = layout(k).1.len();
        let mut colors = vec![Color::BLACK; n];
        colors[0] = Color(0x10, 0x80, 0x20);
        let r = frame(k, &colors);
        assert_eq!(r.len(), 4);
        assert_eq!(r[0][..6], [0x00, 0x7F, 0x01, 60, 0x00, 0x80]);
        assert_eq!(r[1][..4], [0x00, 0x7F, 0x02, 60]);
        assert_eq!(r[2][..4], [0x00, 0x7F, 0x03, 48]);
        assert_eq!(r[3], packet(&[0x00, 0x07, 0x28, 0x01, 0x03, 0x01]));
    }

    #[test]
    fn k63_layout_places_each_key_once() {
        let (_, leds, grid) = layout(Kind::Keyboard(Logical::K63, 0));
        assert_eq!(grid, [19, 6]);
        assert_eq!(leds.len(), tkl_leds().len());
        assert_eq!(leds.len(), 89);
        for (i, a) in leds.iter().enumerate() {
            assert!(a[1] < 1.0 && !leds[i + 1..].contains(a));
        }
    }

    #[test]
    fn dark_core_zone_packets() {
        let r = dark_core(&[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)]);
        assert_eq!(r.len(), 3);
        assert_eq!(
            r[0],
            packet(&[
                0, 0x07, 0xAA, 0, 0, 0x01, 0x07, 0, 0, 100, 1, 2, 3, 0, 0, 0, 5
            ])
        );
        assert_eq!(
            r[1][..17],
            [
                0, 0x07, 0xAA, 0, 0, 0x02, 0x07, 0, 0, 100, 4, 5, 6, 0, 0, 0, 4
            ]
        );
        assert_eq!(r[2][5..17], [0x04, 0x07, 0, 0, 100, 7, 8, 9, 0, 0, 0, 3]);
        assert_eq!(layout(Kind::DarkCore).1.len(), 3);
        assert_eq!(
            lighting_control(Kind::DarkCore),
            lighting_control(Kind::Mouse)
        );
    }

    #[test]
    fn layout_zone_sizes() {
        let n = |k| layout(k).1.len();
        assert_eq!(n(Kind::Keyboard(Logical::Normal, 0)), 113);
        assert_eq!(n(Kind::Keyboard(Logical::K95Plat, 0)), 139);
        assert_eq!(n(Kind::Keyboard(Logical::K95, 0)), 135);
        assert_eq!(n(Kind::Keyboard(Logical::K55, 0)), 3);
        assert_eq!(n(Kind::Stand), 9);
        assert_eq!(KEYS.len(), 113);
        assert_eq!(KEYS_K70_MK2.len(), 116);
        assert_eq!(KEYS_K95_PLAT.len(), 139);
        assert_eq!(KEYS_K95.len(), 135);
    }
}
