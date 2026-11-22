// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, line, matrix};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

const NA: u8 = 0xFF;

const APEX_9_TKL: u16 = 0x1634;
const APEX_9_MINI: u16 = 0x1620;
const APEX_PRO_TKL: u16 = 0x1614;
const APEX_PRO_TKL_2023: u16 = 0x1628;
const APEX_PRO_TKL_2023_WL_1: u16 = 0x1630;
const APEX_PRO_TKL_2023_WL_2: u16 = 0x1632;
const APEX_PRO_TKL_GEN3: u16 = 0x1642;
const APEX_PRO_TKL_GEN3_WL_1: u16 = 0x1644;
const APEX_PRO_TKL_GEN3_WL_2: u16 = 0x1646;
const APEX_7_TKL: u16 = 0x1618;
const APEX_PRO_MINI_GEN3: u16 = 0x1648;
const APEX_PRO_MINI_WL: u16 = 0x1624;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Debug)]
enum Gen {
    Gen1,
    Gen2,
    Gen3,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Apex,
    ApexM,
    TZone,
    EightZone,
    Old,
}

const fn det(name: &'static str, pid: u16, interface: i32) -> Detector {
    Detector {
        name,
        vid: 0x1038,
        pid,
        interface: Some(interface),
        usage_page: None,
        usage: None,
        open,
    }
}

const fn det_ipu(name: &'static str, pid: u16, interface: i32) -> Detector {
    Detector {
        usage_page: Some(0xFFC0),
        usage: Some(1),
        ..det(name, pid, interface)
    }
}

static DETECTORS: [Detector; 22] = [
    det("SteelSeries Apex 3", 0x161A, 3),
    det_ipu("SteelSeries Apex 3 TKL", 0x1622, 1),
    det("SteelSeries Apex 5", 0x161C, 1),
    det("SteelSeries Apex 5", 0x1650, 1),
    det("SteelSeries Apex 7", 0x1612, 1),
    det("SteelSeries Apex 7 TKL", APEX_7_TKL, 1),
    det("SteelSeries Apex 9 TKL", APEX_9_TKL, 1),
    det("SteelSeries Apex 9 Mini", APEX_9_MINI, 1),
    det("SteelSeries Apex Pro", 0x1610, 1),
    det("SteelSeries Apex Pro TKL", APEX_PRO_TKL, 1),
    det("SteelSeries Apex Pro TKL 2023 Wired", APEX_PRO_TKL_2023, 1),
    det_ipu(
        "SteelSeries Apex Pro TKL 2023 Wireless",
        APEX_PRO_TKL_2023_WL_1,
        3,
    ),
    det_ipu(
        "SteelSeries Apex Pro TKL 2023 Wireless",
        APEX_PRO_TKL_2023_WL_2,
        3,
    ),
    det("SteelSeries Apex Pro TKL Gen 3 Wired", APEX_PRO_TKL_GEN3, 1),
    det_ipu(
        "SteelSeries Apex Pro TKL Gen 3 Wireless",
        APEX_PRO_TKL_GEN3_WL_1,
        3,
    ),
    det_ipu(
        "SteelSeries Apex Pro TKL Gen 3 Wireless",
        APEX_PRO_TKL_GEN3_WL_2,
        3,
    ),
    det("SteelSeries Apex M750", 0x0616, 2),
    det("SteelSeries Apex (OG)/Apex Fnatic", 0x1202, 0),
    det("SteelSeries Apex 350", 0x1206, 0),
    det("SteelSeries Apex Pro 3", 0x1640, 1),
    det("SteelSeries Apex Pro Mini Gen 3", APEX_PRO_MINI_GEN3, 1),
    det_ipu("SteelSeries Apex Pro Mini Wireless", APEX_PRO_MINI_WL, 3),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

fn kind(pid: u16) -> Kind {
    match pid {
        0x161A => Kind::TZone,
        0x1622 => Kind::EightZone,
        0x0616 => Kind::ApexM,
        0x1202 | 0x1206 => Kind::Old,
        _ => Kind::Apex,
    }
}

/// Protocol generation by PID.
fn base_gen(pid: u16) -> Gen {
    match pid {
        APEX_9_TKL | APEX_9_MINI | APEX_PRO_TKL_2023 => Gen::Gen2,
        APEX_PRO_TKL_2023_WL_1
        | APEX_PRO_TKL_2023_WL_2
        | APEX_PRO_TKL_GEN3
        | APEX_PRO_TKL_GEN3_WL_1
        | APEX_PRO_TKL_GEN3_WL_2
        | APEX_PRO_MINI_GEN3
        | APEX_PRO_MINI_WL => Gen::Gen3,
        _ => Gen::Gen1,
    }
}

fn direct_len(pid: u16) -> usize {
    if matches!(pid, APEX_9_TKL | APEX_9_MINI) {
        513
    } else {
        643
    }
}

fn wireless(pid: u16) -> bool {
    matches!(
        pid,
        APEX_PRO_TKL_GEN3_WL_1
            | APEX_PRO_TKL_GEN3_WL_2
            | APEX_PRO_TKL_2023_WL_1
            | APEX_PRO_TKL_2023_WL_2
            | APEX_PRO_MINI_WL
    )
}

/// ExtractVersion: "a.b.rest" from the first two dots.
fn extract_version(s: &str) -> String {
    let Some(major) = s.find('.') else {
        return String::new();
    };
    let Some(minor) = s[major + 1..].find('.').map(|i| i + major + 1) else {
        return String::new();
    };
    format!(
        "{}.{}.{}",
        &s[..major],
        &s[major + 1..minor],
        &s[minor + 1..]
    )
}

/// sscanf("%d.%d.%d").
fn scan3(s: &str) -> Option<(i32, i32, i32)> {
    fn int(s: &str) -> Option<(i32, &str)> {
        let s = s.trim_start();
        let digits_at = usize::from(s.starts_with(['-', '+']));
        let end = s[digits_at..]
            .find(|c: char| !c.is_ascii_digit())
            .map_or(s.len(), |i| i + digits_at);
        if end == digits_at {
            return None;
        }
        Some((s[..end].parse().ok()?, &s[end..]))
    }
    let (a, r) = int(s)?;
    let (b, r) = int(r.strip_prefix('.')?)?;
    let (c, _) = int(r.strip_prefix('.')?)?;
    Some((a, b, c))
}

/// GetVersion reply to SendInitialization's GEN3 check (firmware 1.19.7 and later).
fn is_gen3_firmware(reply: Option<&[u8]>) -> bool {
    let version = match reply {
        Some(ibuf) => {
            let mut s: String = ibuf
                .iter()
                .filter(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            if ibuf.first() == Some(&0x90) && !s.is_empty() {
                s.remove(0);
            }
            format!("KBD: {}", extract_version(&s))
        }
        None => "Unsupported protocol".into(),
    };
    let rest: String = version.chars().skip(4).collect();
    match scan3(&rest) {
        Some((major, minor, patch)) => {
            major > 1 || (major == 1 && (minor > 19 || (minor == 19 && patch >= 7)))
        }
        None => false,
    }
}

fn read_version(hid: &HidDevice) -> Option<Vec<u8>> {
    let mut obuf = [0u8; 65];
    obuf[1] = 0x90;
    hid.write(&obuf).ok()?;
    let mut ibuf = [0u8; 64];
    match hid.read_timeout(&mut ibuf, 2) {
        Ok(n) if n > 0 => Some(ibuf.to_vec()),
        _ => None,
    }
}

/// SteelSeriesApexController::SetLEDsDirect.
fn apex_direct(pid: u16, generation: Gen, colors: &[Color]) -> Vec<u8> {
    let len = direct_len(pid);
    let mut n = KEYS.len().min(colors.len());
    if n * 4 + 3 > len {
        n = (len - 3) / 4;
    }
    let id = if generation >= Gen::Gen2 {
        if wireless(pid) { 0x61 } else { 0x40 }
    } else {
        0x3A
    };
    let mut b = vec![0u8; len];
    b[1] = id;
    b[2] = n as u8;
    for (i, c) in colors.iter().take(n).enumerate() {
        b[i * 4 + 3..i * 4 + 7].copy_from_slice(&[KEYS[i], c.0, c.1, c.2]);
    }
    b
}

fn apex_m_enable() -> Vec<Op> {
    let mut a = vec![0u8; 513];
    a[4] = 0x01;
    a[6] = 0x85;
    let mut b = a.clone();
    b[4] = 0x03;
    b[5] = 0x01;
    b[6] = 0x00;
    b[7] = 0xFF;
    // the third report reuses the buffer, so byte 7 keeps 0xFF
    let mut c = b.clone();
    c[4] = 0x01;
    c[5] = 0x00;
    c[6] = 0x85;
    vec![Op::Feature(a), Op::Feature(b), Op::Feature(c)]
}

/// SteelSeriesApexMController::SetLEDsDirect.
fn apex_m_direct(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 513];
    b[3..9].copy_from_slice(&[0x01, 0x8E, 0x01, 0x03, 0x06, 0x16]);
    for (i, &k) in KEYS_M.iter().enumerate() {
        let rgb = if k == NA {
            [0xFF, 0x32, 0x00]
        } else {
            let c = colors.get(k as usize).copied().unwrap_or(Color::BLACK);
            [c.0, c.1, c.2]
        };
        b[i * 3 + 9..i * 3 + 12].copy_from_slice(&rgb);
    }
    b
}

/// SteelSeriesApexTZoneController::SetColor with Direct brightness 0x64.
fn tzone(colors: &[Color]) -> Vec<Op> {
    let mut b = vec![0u8; 33];
    b[1] = 0x0A;
    b[3] = 0x64;
    let first = b.clone();
    b[1] = 0x0B;
    for (i, c) in colors.iter().take(10).enumerate() {
        b[i * 3 + 3..i * 3 + 6].copy_from_slice(&[c.0, c.1, c.2]);
    }
    vec![Op::Write(first), Op::Write(b)]
}

/// SteelSeriesApex8ZoneController::SetColor in Direct (mode 0).
fn eight_zone(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = 0x21;
    b[2] = 0xFF;
    for (i, c) in colors.iter().take(8).enumerate() {
        b[i * 3 + 3..i * 3 + 6].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

/// SteelSeriesOldApexController::SetColorDetailed with alpha = Direct's 0x08.
fn old_apex(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 33];
    b[1] = 0x07;
    for i in 0..5 {
        let c = colors.get(i).copied().unwrap_or(Color::BLACK);
        b[3 + i * 4..7 + i * 4].copy_from_slice(&[c.0, c.1, c.2, 0x08]);
    }
    b
}

struct Keyboard {
    kind: Kind,
    pid: u16,
    generation: Gen,
    started: bool,
}

impl Proto for Keyboard {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let first = !self.started;
        self.started = true;
        match self.kind {
            Kind::Apex => {
                let mut ops = Vec::new();
                if first && self.generation == Gen::Gen3 {
                    ops.push(Op::Feature(padded(65, &[0x00, 0x4B])));
                }
                ops.push(Op::Feature(apex_direct(self.pid, self.generation, colors)));
                ops
            }
            Kind::ApexM => {
                let mut ops = if first { apex_m_enable() } else { Vec::new() };
                ops.push(Op::Feature(apex_m_direct(colors)));
                ops
            }
            Kind::TZone => tzone(colors),
            Kind::EightZone => {
                let mut ops = vec![Op::Write(eight_zone(colors))];
                if first {
                    ops.push(Op::Write(padded(65, &[0x00, 0x23, 0x10])));
                }
                ops
            }
            Kind::Old => vec![Op::Write(old_apex(colors))],
        }
    }

    fn reset(&mut self) {
        self.started = false;
    }
}

fn padded(len: usize, head: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[..head.len()].copy_from_slice(head);
    v
}

/// ANSI keymap with the TKL or Mini patch for these boards' SKUs.
fn apex_layout(pid: u16) -> (Vec<[f32; 2]>, [u32; 2]) {
    let mut map = ANSI;
    let patch: &[(usize, usize, u8)] = match pid {
        APEX_9_MINI | APEX_PRO_MINI_GEN3 | APEX_PRO_MINI_WL => MINI,
        APEX_7_TKL
        | APEX_9_TKL
        | APEX_PRO_TKL
        | APEX_PRO_TKL_2023
        | APEX_PRO_TKL_2023_WL_1
        | APEX_PRO_TKL_2023_WL_2
        | APEX_PRO_TKL_GEN3
        | APEX_PRO_TKL_GEN3_WL_1
        | APEX_PRO_TKL_GEN3_WL_2 => TKL,
        _ => &[],
    };
    for &(r, c, v) in patch {
        map[r * 22 + c] = v;
    }
    let cells: Vec<Option<usize>> = map
        .iter()
        .map(|&v| (v != NA).then_some(v as usize))
        .collect();
    matrix(KEYS.len(), 6, 22, &cells)
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let kind = kind(info.pid);
    let hid = info.open(api)?;
    let mut generation = base_gen(info.pid);
    if kind == Kind::Apex
        && matches!(info.pid, APEX_PRO_TKL | APEX_9_TKL | APEX_9_MINI)
        && is_gen3_firmware(read_version(&hid).as_deref())
    {
        generation = Gen::Gen3;
    }
    let layout = match kind {
        Kind::Apex => apex_layout(info.pid),
        Kind::ApexM => apex_layout(0),
        Kind::TZone => line(10),
        Kind::EightZone => line(8),
        Kind::Old => line(5),
    };
    let dev = device(
        info.device_id("steelseries-apex"),
        det.name,
        DeviceKind::Keyboard,
        layout,
    );
    let proto = Keyboard {
        kind,
        pid: info.pid,
        generation,
        started: false,
    };
    Ok(Driver::boxed(hid, "steelseries apex", dev, proto))
}

const KEYS: [u8; 112] = [
    0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13,
    0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23,
    0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x32, 0x33, 0x34,
    0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44,
    0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x64, 0xE0,
    0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xF0, 0x31, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0x53, 0x54,
    0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x61, 0x62, 0x63, 0xFB,
];
#[rustfmt::skip]
const ANSI: [u8; 132] = [
    37, NA, 53, 54, 55, 56, NA, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, NA, NA, NA, NA,
    48, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 41, 42, 38, NA, 68, 69, 70, 94, 95, 96, 97,
    39, NA, 16, 22, 4, 17, 19, 24, 20, 8, 14, 15, 43, 44, 88, 71, 72, 73, 106, 107, 108, 98,
    52, NA, 0, 18, 3, 5, 6, 7, 9, 10, 11, 46, 47, 36, NA, NA, NA, NA, 103, 104, 105, NA,
    80, NA, 25, 23, 2, 21, 1, 13, 12, 49, 50, 51, 84, NA, NA, NA, 77, NA, 100, 101, 102, 99,
    79, 82, 81, NA, NA, NA, NA, 40, NA, NA, NA, 85, 86, 87, 83, 75, 76, 74, 109, NA, 110, NA,
];
const TKL: &[(usize, usize, u8)] = &[
    (0, 15, NA),
    (0, 16, NA),
    (0, 17, 111),
    (1, 18, NA),
    (1, 19, NA),
    (1, 20, NA),
    (1, 21, NA),
    (2, 18, NA),
    (2, 19, NA),
    (2, 20, NA),
    (2, 21, NA),
    (3, 18, NA),
    (3, 19, NA),
    (3, 20, NA),
    (4, 18, NA),
    (4, 19, NA),
    (4, 20, NA),
    (4, 21, NA),
    (5, 18, NA),
    (5, 20, NA),
];
const MINI: &[(usize, usize, u8)] = &[
    (0, 0, NA),
    (0, 2, NA),
    (0, 3, NA),
    (0, 4, NA),
    (0, 5, NA),
    (0, 7, NA),
    (0, 8, NA),
    (0, 9, NA),
    (0, 10, NA),
    (0, 11, NA),
    (0, 12, NA),
    (0, 13, NA),
    (0, 14, NA),
    (0, 15, NA),
    (0, 16, NA),
    (0, 17, NA),
    (1, 0, 37),
    (1, 15, NA),
    (1, 16, NA),
    (1, 17, NA),
    (1, 18, NA),
    (1, 19, NA),
    (1, 20, NA),
    (1, 21, NA),
    (2, 15, NA),
    (2, 16, NA),
    (2, 17, NA),
    (2, 18, NA),
    (2, 19, NA),
    (2, 20, NA),
    (2, 21, NA),
    (3, 18, NA),
    (3, 19, NA),
    (3, 20, NA),
    (4, 16, NA),
    (4, 18, NA),
    (4, 19, NA),
    (4, 20, NA),
    (4, 21, NA),
    (5, 15, NA),
    (5, 16, NA),
    (5, 17, NA),
    (5, 18, NA),
    (5, 20, NA),
];
#[rustfmt::skip]
const KEYS_M: [u8; 132] = [
    79, 82, 81, NA, 40, NA, NA, NA, NA, 85, 86, 87, 83, NA, NA, 75, 76, 74, NA, 104, NA, 105,
    80, 25, 23, 2, 21, 1, 13, 12, 49, 50, 51, NA, 84, NA, NA, NA, 77, NA, 95, 96, 97, 94,
    52, 0, 18, 3, 5, 6, 7, 9, 10, 11, 46, 47, NA, 36, NA, NA, NA, NA, 98, 99, 100, NA,
    39, 16, 22, 4, 17, 19, 24, 20, 8, 14, 15, 43, 44, NA, 88, 71, 72, 73, 101, 102, 103, 93,
    48, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 41, 42, NA, 38, 68, 69, 70, 89, 90, 91, 92,
    37, 53, 54, 55, 56, NA, 57, 58, 59, 60, NA, 61, 62, 63, 64, 65, 66, 67, NA, NA, NA, NA,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(op: &Op) -> &[u8] {
        match op {
            Op::Write(b) | Op::Feature(b) => b,
            other => panic!("{other:?}"),
        }
    }

    // Known keyboard ids plus the Apex 5, Pro Mini Gen 3 and Pro Mini Wireless.
    #[test]
    fn detectors() {
        assert_eq!(DETECTORS.len(), 22);
        let wl = DETECTORS
            .iter()
            .filter(|d| d.usage_page == Some(0xFFC0))
            .count();
        assert_eq!(wl, 6);
    }

    // SetLEDsDirect on a GEN1 Apex 7: 643-byte feature report, packet id 0x3A, 112 keys.
    #[test]
    fn apex_gen1_frame() {
        let colors: Vec<Color> = (0..112).map(|i| Color(i as u8, 1, 2)).collect();
        let b = apex_direct(0x1612, Gen::Gen1, &colors);
        assert_eq!(b.len(), 643);
        assert_eq!(b[..7], [0x00, 0x3A, 112, 0x04, 0, 1, 2]);
        assert_eq!(b[3 + 111 * 4..3 + 112 * 4], [0xFB, 111, 1, 2]);
        assert_eq!(b[3 + 88 * 4], 0x31);
        assert!(b[3 + 112 * 4..].iter().all(|&x| x == 0));
    }

    // GEN3 wireless: SendInitialization (0x4B) once, then 0x61 frames; Apex 9: 513 bytes, 0x40.
    #[test]
    fn apex_gen3_wireless_and_apex9() {
        let mut k = Keyboard {
            kind: Kind::Apex,
            pid: APEX_PRO_TKL_GEN3_WL_1,
            generation: Gen::Gen3,
            started: false,
        };
        let colors = vec![Color(1, 1, 1); 112];
        let ops = k.frame(&colors);
        assert_eq!(ops[0], Op::Feature(padded(65, &[0, 0x4B])));
        assert_eq!(bytes(&ops[1])[..3], [0, 0x61, 112]);
        assert_eq!(k.frame(&colors).len(), 1);
        let b = apex_direct(APEX_9_TKL, Gen::Gen2, &colors);
        assert_eq!(b.len(), 513);
        assert_eq!(b[..3], [0, 0x40, 112]);
    }

    // SendInitialization's firmware check, through GetVersion and ExtractVersion.
    #[test]
    fn gen3_firmware_check() {
        let reply = |s: &str| {
            let mut v = vec![0x90u8];
            v.extend_from_slice(s.as_bytes());
            v.resize(64, 0);
            v
        };
        assert!(is_gen3_firmware(Some(&reply("1.19.7"))));
        assert!(is_gen3_firmware(Some(&reply("2.0.0"))));
        assert!(!is_gen3_firmware(Some(&reply("1.19.6"))));
        assert!(!is_gen3_firmware(Some(&reply("1.5"))));
        assert!(!is_gen3_firmware(None));
        assert_eq!(base_gen(APEX_PRO_TKL), Gen::Gen1);
    }

    // SteelSeriesApexMController: EnableLEDControl, then SetLEDsDirect.
    #[test]
    fn apex_m_frames() {
        let mut k = Keyboard {
            kind: Kind::ApexM,
            pid: 0x0616,
            generation: Gen::Gen1,
            started: false,
        };
        let mut colors = vec![Color::BLACK; 112];
        colors[79] = Color(5, 6, 7); // LCTRL, first key in keys_m
        let ops = k.frame(&colors);
        assert_eq!(ops.len(), 4);
        assert_eq!(bytes(&ops[0])[..8], [0, 0, 0, 0, 0x01, 0, 0x85, 0]);
        assert_eq!(bytes(&ops[1])[..8], [0, 0, 0, 0, 0x03, 0x01, 0x00, 0xFF]);
        assert_eq!(bytes(&ops[2])[..9], [0, 0, 0, 0, 0x01, 0, 0x85, 0xFF, 0]);
        assert!(bytes(&ops[2])[8..].iter().all(|&x| x == 0));
        let d = bytes(&ops[3]);
        assert_eq!(d.len(), 513);
        assert_eq!(
            d[..15],
            [
                0, 0, 0, 0x01, 0x8E, 0x01, 0x03, 0x06, 0x16, 5, 6, 7, 0, 0, 0
            ]
        );
        // keys_m[3] is a gap
        assert_eq!(d[18..21], [0xFF, 0x32, 0x00]);
        assert!(d[9 + 132 * 3..].iter().all(|&x| x == 0));
        assert_eq!(k.frame(&colors).len(), 1);
    }

    // SteelSeriesApexTZoneController::SetColor.
    #[test]
    fn tzone_frame() {
        let colors: Vec<Color> = (0..10).map(|i| Color(i, i, i)).collect();
        let ops = tzone(&colors);
        assert_eq!(ops[0], Op::Write(padded(33, &[0, 0x0A, 0, 0x64])));
        let b = bytes(&ops[1]);
        assert_eq!(b.len(), 33);
        assert_eq!(b[..6], [0, 0x0B, 0, 0, 0, 0]);
        assert_eq!(b[30..33], [9, 9, 9]);
    }

    // SteelSeriesApex8ZoneController::SetColor, brightness sent on the first frame.
    #[test]
    fn eight_zone_frame() {
        let mut k = Keyboard {
            kind: Kind::EightZone,
            pid: 0x1622,
            generation: Gen::Gen1,
            started: false,
        };
        let colors: Vec<Color> = (1..=8).map(|i| Color(i, 0, 0)).collect();
        let ops = k.frame(&colors);
        let b = bytes(&ops[0]);
        assert_eq!(b.len(), 65);
        assert_eq!(b[..6], [0, 0x21, 0xFF, 1, 0, 0]);
        assert_eq!(b[24..27], [8, 0, 0]);
        assert_eq!(ops[1], Op::Write(padded(65, &[0, 0x23, 0x10])));
        assert_eq!(k.frame(&colors).len(), 1);
    }

    // SteelSeriesOldApexController::SetColorDetailed.
    #[test]
    fn old_apex_frame() {
        let colors: Vec<Color> = (1..=5).map(|i| Color(i, i + 10, i + 20)).collect();
        let b = old_apex(&colors);
        assert_eq!(b.len(), 33);
        assert_eq!(
            b[..23],
            [
                0, 0x07, 0x00, 1, 11, 21, 8, 2, 12, 22, 8, 3, 13, 23, 8, 4, 14, 24, 8, 5, 15, 25, 8
            ]
        );
    }

    #[test]
    fn layouts_cover_every_key() {
        assert_eq!(apex_layout(0x1612).0.len(), 112);
        assert_eq!(apex_layout(APEX_9_MINI).0.len(), 112);
    }
}
