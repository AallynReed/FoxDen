//! Ported from OpenRGB Controllers/GigabyteRGBFusion2USBController (GPL-2.0-or-later).

use super::{Driver, Op, Plan};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

const NAME: &str = "Gigabyte RGB Fusion 2 USB";
const IT8297_VID: u16 = 0x048D;
const GC_USB_VID: u16 = 0x0414;

const fn detector(vid: u16, pid: u16) -> Detector {
    Detector {
        name: NAME,
        vid,
        pid,
        interface: None,
        usage_page: Some(0xFF89),
        usage: Some(0xCC),
        open,
    }
}

pub static DETECTORS: [Detector; 5] = [
    detector(IT8297_VID, 0x8297),
    detector(IT8297_VID, 0x8950),
    detector(IT8297_VID, 0x5702),
    detector(IT8297_VID, 0x5711),
    detector(GC_USB_VID, 0xA100),
];

const REPORT_ID: u8 = 0xCC;
const LEN: usize = 64;
const EFFECT_STATIC: u8 = 1;
const BRIGHTNESS_MAX: u8 = 255;
/// OpenRGB starts addressable headers at 0 LEDs; 32 keeps the same LED-count register value.
const DEFAULT_ARGB_LEDS: u16 = 32;

const LED4: u8 = 3;
const HDR_D_LED1: u8 = 5;
const HDR_D_LED2: u8 = 6;
const HDR_D_LED3: u8 = 7;
const HDR_D_LED4: u8 = 8;
const LED10: u8 = 9;
const LED11: u8 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub idx: u8,
    pub min: u16,
    pub max: u16,
    pub leds: u16,
}

const fn z(idx: u8, min: u16, max: u16) -> Zone {
    let leds = if DEFAULT_ARGB_LEDS < min {
        min
    } else if DEFAULT_ARGB_LEDS > max {
        max
    } else {
        DEFAULT_ARGB_LEDS
    };
    Zone {
        idx,
        min,
        max,
        leds,
    }
}

const fn led(n: u8) -> Zone {
    z(n - 1, 1, 1)
}

/// The generic layouts OpenRGB falls back to when the board name is not in its table.
const IT8297: [Zone; 8] = [
    z(HDR_D_LED1, 0, 1024),
    z(HDR_D_LED2, 0, 1024),
    led(1),
    led(2),
    led(3),
    led(4),
    led(5),
    led(8),
];
const IT8297_2: [Zone; 8] = [
    led(1),
    led(2),
    led(3),
    led(4),
    led(5),
    led(6),
    led(7),
    led(8),
];
const IT8950: [Zone; 2] = [z(2, 0, 256), z(LED4, 0, 256)];
const IT5711: [Zone; 12] = [
    z(HDR_D_LED1, 0, 256),
    z(HDR_D_LED2, 0, 256),
    z(HDR_D_LED3, 0, 256),
    z(HDR_D_LED4, 0, 256),
    led(1),
    led(2),
    led(3),
    led(4),
    led(5),
    led(9),
    led(10),
    led(11),
];
const IT5711_2: [Zone; 8] = [
    led(1),
    led(2),
    led(3),
    led(4),
    led(5),
    led(9),
    led(10),
    led(11),
];
const GC_USB: [Zone; 1] = [z(0, 0, 1024)];
const IT5711_LAYOUT_ID: u32 = 0x0000_01DF;

/// RGBController_RGBFusion2USB::Init_Controller fallback choice.
pub fn layout(pid: u16, device_num: u8) -> Vec<Zone> {
    match (device_num, pid) {
        (1, 0x5711) => IT5711_2.to_vec(),
        (1, _) => IT8297_2.to_vec(),
        (_, 0x8950) => IT8950.to_vec(),
        (_, 0x5711) => IT5711.to_vec(),
        (_, 0xA100) => GC_USB.to_vec(),
        _ => IT8297.to_vec(),
    }
}

/// GetIT5711ZoneCounts: LED10 and LED11 counts by firmware series.
pub fn it5711_counts(fw_id: u8, rev: u8, lid: u32) -> (u16, u16) {
    let default_0c = |fallback| match lid {
        0x017001DF => (16, 4),
        0x028001DF => (26, 18),
        _ => fallback,
    };
    match fw_id {
        0x00 => (27, 2),
        0x02 if rev == 0x20 => match lid {
            0x013001DF => (13, 6),
            0x015001DF => (6, 6),
            0x026001DF => (14, 6),
            _ => (0, 0),
        },
        0x02 => match lid {
            0x013001DF => (4, 6),
            0x015001DF => (6, 6),
            _ => (14, 6),
        },
        0x03 | 0x08 => (13, 0),
        0x05 => default_0c((8, 4)),
        0x06 if matches!(rev, 0x19 | 0x1D | 0x24) && matches!(lid, 0x013001DF | 0x311001DF) => {
            (12, 9)
        }
        0x06 => (12, 0),
        0x09 if matches!(rev, 0x19 | 0x1A) && matches!(lid, 0x013001DF | 0x311001DF) => (12, 9),
        0x09 => (12, 0),
        0x0B if rev == 0x05 => (26, 18),
        0x0C => match rev {
            0x06 => default_0c((8, 4)),
            0x08 => default_0c((10, 4)),
            0x09 if lid == 0x016001DF => (10, 4),
            0x09 => default_0c((0, 0)),
            _ => (0, 0),
        },
        0x0D if rev == 0x05 => match lid {
            0x016001DF => (8, 4),
            _ => default_0c((0, 0)),
        },
        0x17 if rev == 0x01 && lid == 0x016001DF => (9, 5),
        _ => (0, 0),
    }
}

/// ApplyIT5711ZoneCounts: a known count fixes LED10/LED11 to that many LEDs.
pub fn apply_it5711_counts(zones: &mut [Zone], fw_id: u8, rev: u8) {
    let (c10, c11) = it5711_counts(fw_id, rev, IT5711_LAYOUT_ID);
    for zone in zones {
        let n = match zone.idx {
            LED10 => c10,
            LED11 => c11,
            _ => continue,
        };
        if n != 0 {
            *zone = z(zone.idx, n, n);
        }
    }
}

/// Each addressable header at the user's length, within its zone limits.
pub fn sized(id: &str, zones: &mut [Zone]) {
    for (n, (i, zone)) in zones
        .iter_mut()
        .enumerate()
        .filter(|(_, z)| z.min != z.max)
        .enumerate()
    {
        let name = format!("Header {}", n + 1);
        let (min, max) = (zone.min as usize, zone.max as usize);
        zone.leds = crate::lengths::length(id, i, &name, zone.leds as usize, min, max) as u16;
    }
}

fn fixed(zone: &Zone) -> bool {
    zone.min == 1 && zone.max == 1
}

pub fn zone_leds(zone: &Zone) -> usize {
    zone.leds as usize
}

/// The IT8297Report fields the driver uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub device_num: u8,
    pub fw_ver: u32,
    pub argb_count: [u8; 6],
    pub support_cmd_flag: u8,
    pub cal: [u32; 6],
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// LedCountFromEnum
fn count_enum(v: u8) -> u8 {
    match v & 0x0F {
        n @ 0..=3 => n,
        _ => 4,
    }
}

/// RefreshHardwareInfo: the CC 60 answer, and CC 61 for strips 2..5 on the IT5711.
pub fn parse_report(b: &[u8], cal5711: Option<&[u8]>) -> Option<Report> {
    if b.len() < LEN {
        return None;
    }
    let mut cal = [u32le(b, 44), u32le(b, 48), 0, 0, 0, 0];
    if let Some(c) = cal5711.filter(|c| c.len() >= LEN) {
        for i in 0..4 {
            cal[2 + i] = u32le(c, 4 + i * 4);
        }
    }
    Some(Report {
        device_num: b[2],
        fw_ver: u32le(b, 4),
        argb_count: [
            count_enum(b[8]),
            count_enum(b[8] >> 4),
            count_enum(b[9]),
            count_enum(b[9] >> 4),
            count_enum(b[10]),
            count_enum(b[10] >> 4),
        ],
        support_cmd_flag: b[11],
        cal,
    })
}

/// SendCCReport
pub fn cc(a: u8, b: u8, c: u8) -> Vec<u8> {
    let mut p = vec![0u8; LEN];
    p[..4].copy_from_slice(&[REPORT_ID, a, b, c]);
    p
}

/// LedCountToEnum
fn led_count_enum(c: usize) -> u8 {
    match c {
        0..=32 => 0,
        33..=64 => 1,
        65..=256 => 2,
        257..=512 => 3,
        _ => 4,
    }
}

/// Which of the six LED-count slots a header fills (SetupZones d1..d6).
fn count_slot(idx: u8) -> usize {
    match idx {
        LED4 | HDR_D_LED2 => 1,
        HDR_D_LED3 => 2,
        HDR_D_LED4 => 3,
        LED10 => 4,
        LED11 => 5,
        _ => 0,
    }
}

/// PktRGB::Init header remap.
pub fn argb_header(idx: u8) -> u8 {
    match idx {
        LED4 | HDR_D_LED2 => 0x59,
        HDR_D_LED3 => 0x62,
        HDR_D_LED4 => 0x63,
        LED10 => 0x64,
        LED11 => 0x65,
        _ => 0x58,
    }
}

/// SetStripBuiltinEffectState bit for one header, or every header when `None`.
fn strip_bit(pid: u16, idx: Option<u8>) -> i32 {
    match idx {
        None => match pid {
            0xA100 => 0x01,
            0x5711 => 0x01 | 0x02 | 0x08 | 0x10 | 0x20 | 0x40,
            _ => 0x01 | 0x02,
        },
        Some(LED4 | HDR_D_LED2) => 0x02,
        Some(HDR_D_LED3) => 0x08,
        Some(HDR_D_LED4) => 0x10,
        Some(LED10) => 0x20,
        Some(LED11) => 0x40,
        Some(_) => 0x01,
    }
}

/// PktEffect::Init plus SetLEDEffect for EFFECT_STATIC; returns the zone bit too.
pub fn static_effect(led: u8, pid: u16, brightness: u8, c: Color) -> (Vec<u8>, u32) {
    let (zone0, header) = if pid == 0xA100 {
        (0xFF, 0x20)
    } else if led < 8 {
        (1u32 << led, 0x20 + led)
    } else if led < 11 {
        (1u32 << led, 0x90 + (led - 8))
    } else {
        (0, 0)
    };
    let mut p = vec![0u8; LEN];
    p[0] = REPORT_ID;
    p[1] = header;
    p[2..6].copy_from_slice(&zone0.to_le_bytes());
    p[11] = EFFECT_STATIC;
    p[12] = brightness;
    p[14..18].copy_from_slice(&[c.2, c.1, c.0, 0]);
    (p, zone0)
}

/// ApplyEffect without fast apply.
pub fn apply(mask: u32) -> Vec<u8> {
    let mut p = vec![0u8; LEN];
    p[0] = REPORT_ID;
    p[1] = 0x28;
    p[2..6].copy_from_slice(&mask.to_le_bytes());
    p
}

/// SetStripColors: 19 LEDs per packet in calibrated byte order; the buffer is reused between packets.
pub fn strip_colors(idx: u8, colors: &[Color], cal: &[u32; 6]) -> Vec<Vec<u8>> {
    let header = argb_header(idx);
    let order = match header {
        0x59 => cal[1],
        0x62 => cal[2],
        0x63 => cal[3],
        0x64 => cal[4],
        0x65 => cal[5],
        _ => cal[0],
    };
    let (bo_r, bo_g, bo_b) = (
        (order >> 16) as u8 as usize,
        (order >> 8) as u8 as usize,
        order as u8 as usize,
    );
    let mut p = vec![0u8; LEN];
    p[0] = REPORT_ID;
    p[1] = header;
    let mut out = Vec::new();
    let mut sent = 0usize;
    for chunk in colors.chunks(19) {
        let count = chunk.len() * 3;
        p[2..4].copy_from_slice(&(sent as u16).to_le_bytes());
        p[4] = count as u8;
        sent += count;
        for (i, c) in chunk.iter().enumerate() {
            let at = i * 3 + 5;
            p[at + bo_r] = c.0;
            p[at + bo_g] = c.1;
            p[at + bo_b] = c.2;
        }
        out.push(p.clone());
    }
    out
}

pub struct Board {
    pid: u16,
    report: Report,
    zones: Vec<Zone>,
    effect_disabled: i32,
    mask: u32,
}

impl Board {
    pub fn new(pid: u16, report: Report) -> Self {
        let mut zones = layout(pid, report.device_num);
        if pid == 0x5711 {
            apply_it5711_counts(
                &mut zones,
                (report.fw_ver >> 24) as u8,
                (report.fw_ver >> 16) as u8,
            );
        }
        Board {
            pid,
            report,
            zones,
            effect_disabled: -1,
            mask: 0,
        }
    }

    fn led_count(&self) -> usize {
        self.zones.iter().map(zone_leds).sum()
    }

    fn strip_state_off(&mut self, idx: Option<u8>, ops: &mut Vec<Op>) {
        let new = self.effect_disabled.max(0) | strip_bit(self.pid, idx);
        if self.effect_disabled >= 0 && new == self.effect_disabled {
            return;
        }
        self.effect_disabled = new;
        ops.push(Op::Feature(cc(0x32, new as u8, 0)));
        ops.push(Op::Sleep(50));
    }
}

impl Plan for Board {
    /// The controller constructor's init, then SetupZones' LED counts and effect state.
    fn first(&mut self, _: &[Color]) -> Vec<Op> {
        self.effect_disabled = -1;
        self.mask = 0;
        let mut ops = Vec::new();
        if self.report.support_cmd_flag & 0x02 != 0 {
            ops.push(Op::Feature(cc(0x48, 0, 0)));
        }
        for reg in 0x20..=0x27 {
            ops.push(Op::Feature(cc(reg, 0, 0)));
        }
        if self.pid == 0x5711 {
            for reg in 0x90..=0x92 {
                ops.push(Op::Feature(cc(reg, 0, 0)));
            }
        }
        let fast = if self.pid == 0x5711 { 0x07 } else { 0x00 };
        ops.push(Op::Feature(cc(0x28, 0xFF, fast)));
        ops.push(Op::Feature(cc(0x31, 0, 0)));
        let mut counts = [0usize; 6];
        for zone in self.zones.iter().filter(|z| !fixed(z)) {
            counts[count_slot(zone.idx)] = zone_leds(zone);
        }
        let e = counts.map(led_count_enum);
        if e != self.report.argb_count {
            self.report.argb_count = e;
            let mut p = vec![0u8; LEN];
            p[..5].copy_from_slice(&[
                REPORT_ID,
                0x34,
                (e[1] << 4) | e[0],
                (e[3] << 4) | e[2],
                (e[5] << 4) | e[4],
            ]);
            ops.push(Op::Feature(p));
        }
        self.strip_state_off(None, &mut ops);
        ops
    }

    /// DeviceUpdateLEDs in Direct mode.
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        let mut at = 0;
        for zone in self.zones.clone() {
            let n = zone_leds(&zone);
            let c = &colors[at..at + n];
            at += n;
            if fixed(&zone) {
                let (p, bit) = static_effect(zone.idx, self.pid, BRIGHTNESS_MAX, c[0]);
                self.mask |= bit;
                ops.push(Op::Feature(p));
            } else if n > 0 {
                self.strip_state_off(Some(zone.idx), &mut ops);
                ops.extend(
                    strip_colors(zone.idx, c, &self.report.cal)
                        .into_iter()
                        .map(Op::Feature),
                );
            }
        }
        ops.push(Op::Feature(apply(self.mask)));
        self.mask = 0;
        ops
    }
}

fn get_feature(hid: &HidDevice, cmd: u8) -> Result<Vec<u8>, String> {
    hid.send_feature_report(&cc(cmd, 0, 0))
        .map_err(|e| e.to_string())?;
    let mut b = vec![0u8; LEN];
    b[0] = REPORT_ID;
    let n = hid.get_feature_report(&mut b).map_err(|e| e.to_string())?;
    b.truncate(n);
    Ok(b)
}

fn device(id: String, name: &str, zones: &[Zone]) -> Device {
    let strips = zones.iter().filter(|z| !fixed(z)).count();
    let singles = zones.len() - strips;
    let rows = strips + usize::from(singles > 0);
    let cols = zones
        .iter()
        .map(zone_leds)
        .max()
        .unwrap_or(1)
        .max(singles)
        .max(1);
    let mut leds = Vec::new();
    let (mut single, mut strip) = (0, usize::from(singles > 0));
    for zone in zones {
        let y = |r: usize| (r as f32 + 0.5) / rows as f32;
        if fixed(zone) {
            leds.push([(single as f32 + 0.5) / cols as f32, y(0)]);
            single += 1;
        } else {
            let r = y(strip);
            leds.extend((0..zone_leds(zone)).map(|i| [(i as f32 + 0.5) / cols as f32, r]));
            strip += 1;
        }
    }
    Device {
        id,
        name: name.into(),
        kind: DeviceKind::Motherboard,
        rect: [0.06, 0.12, 0.52, 0.04 * rows as f32],
        grid: [cols as u32, rows as u32],
        leds,
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let main = get_feature(&hid, 0x60)?;
    let extra = if info.pid == 0x5711 {
        get_feature(&hid, 0x61).ok()
    } else {
        None
    };
    let report = parse_report(&main, extra.as_deref()).ok_or("short IT8297 report")?;
    let mut board = Board::new(info.pid, report);
    let id = info.device_id("gigabyte-fusion2");
    sized(&id, &mut board.zones);
    let name = if info.pid == 0xA100 { "GC-USB" } else { d.name };
    let device = device(id, name, &board.zones);
    if device.leds.len() != board.led_count() {
        return Err("layout mismatch".into());
    }
    Ok(Driver::boxed(
        "gigabyte rgb fusion 2 usb",
        device,
        hid,
        board,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    // "GRB" in GigabyteCalibrationsLookup, as EncodeCalibrationBuffer packs it.
    const GRB: u32 = 0x0001_0002;

    fn report_bytes() -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0] = 0xCC;
        b[2] = 0x00;
        b[4..8].copy_from_slice(&[0x0E, 0x00, 0x20, 0x02]);
        b[8] = 0x10;
        b[9] = 0x00;
        b[10] = 0x00;
        b[11] = 0x03;
        b[44..48].copy_from_slice(&GRB.to_le_bytes());
        b[48..52].copy_from_slice(&0x0002_0001u32.to_le_bytes());
        b
    }

    // IT8297Report layout read by RefreshHardwareInfo.
    #[test]
    fn report_fields() {
        let mut c = vec![0u8; 64];
        c[4..8].copy_from_slice(&7u32.to_le_bytes());
        c[16..20].copy_from_slice(&9u32.to_le_bytes());
        let r = parse_report(&report_bytes(), Some(&c)).unwrap();
        assert_eq!(r.device_num, 0);
        assert_eq!(r.fw_ver, 0x0220_000E);
        assert_eq!(r.argb_count, [0, 1, 0, 0, 0, 0]);
        assert_eq!(r.support_cmd_flag, 3);
        assert_eq!(r.cal, [GRB, 0x0002_0001, 7, 0, 0, 9]);
        assert!(parse_report(&[0xCC; 63], None).is_none());
    }

    #[test]
    fn layouts_follow_init_controller() {
        assert_eq!(layout(0x8297, 0), IT8297.to_vec());
        assert_eq!(layout(0x5702, 1), IT8297_2.to_vec());
        assert_eq!(layout(0xA100, 1), IT8297_2.to_vec());
        assert_eq!(layout(0x8950, 0), IT8950.to_vec());
        assert_eq!(layout(0x5711, 0), IT5711.to_vec());
        assert_eq!(layout(0x5711, 1), IT5711_2.to_vec());
        assert_eq!(layout(0xA100, 0), GC_USB.to_vec());
    }

    // GetIT5711ZoneCounts / ApplyIT5711ZoneCounts with the generic layout id.
    #[test]
    fn it5711_onboard_counts() {
        assert_eq!(it5711_counts(0x00, 0, IT5711_LAYOUT_ID), (27, 2));
        assert_eq!(it5711_counts(0x02, 0x20, IT5711_LAYOUT_ID), (0, 0));
        assert_eq!(it5711_counts(0x02, 0x10, IT5711_LAYOUT_ID), (14, 6));
        assert_eq!(it5711_counts(0x03, 0, IT5711_LAYOUT_ID), (13, 0));
        assert_eq!(it5711_counts(0x05, 0, 0x028001DF), (26, 18));
        assert_eq!(it5711_counts(0x06, 0x19, 0x013001DF), (12, 9));
        assert_eq!(it5711_counts(0x0C, 0x09, 0x016001DF), (10, 4));
        assert_eq!(it5711_counts(0x0C, 0x09, IT5711_LAYOUT_ID), (0, 0));
        assert_eq!(it5711_counts(0x0D, 0x05, 0x017001DF), (16, 4));
        let mut zones = IT5711.to_vec();
        apply_it5711_counts(&mut zones, 0x03, 0);
        assert_eq!(zones[10], z(LED10, 13, 13));
        assert_eq!(zones[11], z(LED11, 1, 1));
    }

    // SendCCReport(0x60, 0x00)
    #[test]
    fn cc_report() {
        let mut e = vec![0u8; 64];
        e[0] = 0xCC;
        e[1] = 0x60;
        assert_eq!(cc(0x60, 0, 0), e);
    }

    // RGBFusion2USBController::SetLEDEffect(led, EFFECT_STATIC, 0, 255, false, color)
    #[test]
    fn static_effect_packets() {
        let (p, bit) = static_effect(0, 0x8297, 0xFF, Color(0x11, 0x22, 0x33));
        let mut e = vec![0u8; 64];
        e[..18].copy_from_slice(&[
            0xCC, 0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x01, 0xFF, 0x00, 0x33, 0x22, 0x11, 0x00,
        ]);
        assert_eq!(p, e);
        assert_eq!(bit, 1);
        let (p, bit) = static_effect(LED10, 0x5711, 0xFF, Color(1, 2, 3));
        assert_eq!(p[..6], [0xCC, 0x91, 0x00, 0x02, 0x00, 0x00]);
        assert_eq!(bit, 0x200);
        let (p, bit) = static_effect(3, 0xA100, 0xFF, Color(1, 2, 3));
        assert_eq!(p[..6], [0xCC, 0x20, 0xFF, 0x00, 0x00, 0x00]);
        assert_eq!(bit, 0xFF);
    }

    // RGBFusion2USBController::SetStripColors: partial last packet keeps stale bytes.
    #[test]
    fn strip_packets() {
        let colors: Vec<Color> = (0..20u8).map(|i| Color(i, 0x40 + i, 0x80 + i)).collect();
        let p = strip_colors(HDR_D_LED1, &colors, &[GRB, 0, 0, 0, 0, 0]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..5], [0xCC, 0x58, 0x00, 0x00, 57]);
        // GRB: green, red, blue on the wire
        assert_eq!(p[0][5..8], [0x40, 0x00, 0x80]);
        assert_eq!(p[0][59..62], [0x52, 0x12, 0x92]);
        assert_eq!(p[0][62..], [0, 0]);
        assert_eq!(p[1][..5], [0xCC, 0x58, 57, 0x00, 3]);
        assert_eq!(p[1][5..8], [0x53, 0x13, 0x93]);
        assert_eq!(p[1][8..11], [0x41, 0x01, 0x81]);
        let p = strip_colors(HDR_D_LED2, &colors[..1], &[0, 0x0002_0001, 0, 0, 0, 0]);
        // byte order 2/0/1 on the second header: green, blue, red
        assert_eq!(p[0][..8], [0xCC, 0x59, 0, 0, 3, 0x40, 0x80, 0x00]);
    }

    // ApplyEffect(false) after a direct frame.
    #[test]
    fn apply_packet() {
        let mut e = vec![0u8; 64];
        e[..6].copy_from_slice(&[0xCC, 0x28, 0x9F, 0x00, 0x00, 0x00]);
        assert_eq!(apply(0x9F), e);
    }

    // Controller constructor, SetupZones (SetLedCount, SetStripBuiltinEffectState(-1)), then DeviceUpdateLEDs.
    #[test]
    fn it8297_first_push_and_frame() {
        let report = parse_report(&report_bytes(), None).unwrap();
        let board = Board::new(0x8297, report);
        let leds = board.led_count();
        assert_eq!(leds, 32 + 32 + 6);
        let mut s = Session::new(board, leds);
        let mut colors = vec![Color(9, 8, 7); leds];
        colors[64] = Color(0xAA, 0xBB, 0xCC);
        let ops = s.step(&colors);
        let mut want = vec![Op::Feature(cc(0x48, 0, 0))];
        want.extend((0x20..=0x27).map(|r| Op::Feature(cc(r, 0, 0))));
        want.push(Op::Feature(cc(0x28, 0xFF, 0x00)));
        want.push(Op::Feature(cc(0x31, 0, 0)));
        // report said 64 on D_LED2; both headers now at 32
        let mut counts = vec![0u8; 64];
        counts[..5].copy_from_slice(&[0xCC, 0x34, 0x00, 0x00, 0x00]);
        want.push(Op::Feature(counts));
        want.push(Op::Feature(cc(0x32, 0x03, 0)));
        want.push(Op::Sleep(50));
        want.extend(
            strip_colors(HDR_D_LED1, &colors[..32], &s.plan.report.cal)
                .into_iter()
                .map(Op::Feature),
        );
        want.extend(
            strip_colors(HDR_D_LED2, &colors[32..64], &s.plan.report.cal)
                .into_iter()
                .map(Op::Feature),
        );
        for (i, idx) in [0u8, 1, 2, 3, 4, 7].into_iter().enumerate() {
            want.push(Op::Feature(
                static_effect(idx, 0x8297, 0xFF, colors[64 + i]).0,
            ));
        }
        want.push(Op::Feature(apply(0x9F)));
        assert_eq!(ops, want);
        assert_eq!(ops.len(), 8 + 1 + 1 + 1 + 1 + 2 + 2 + 2 + 6 + 1);

        // the next frame has no setup and no repeated effect-state write
        colors[0] = Color(0, 0, 0);
        let ops = s.step(&colors);
        assert_eq!(ops.len(), 2 + 2 + 6 + 1);
        assert!(matches!(&ops[0], Op::Feature(p) if p[1] == 0x58));
        assert_eq!(ops.last(), Some(&Op::Feature(apply(0x9F))));
        assert!(s.step(&colors).is_empty());
    }

    // IT5711 reset adds registers 0x90..0x92 and the 0x07 fast apply; the strip mask covers six headers.
    #[test]
    fn it5711_first_push() {
        let mut b = report_bytes();
        b[8] = 0x00;
        b[11] = 0x01;
        b[4..8].copy_from_slice(&0x0300_0000u32.to_le_bytes());
        let board = Board::new(0x5711, parse_report(&b, None).unwrap());
        assert_eq!(board.led_count(), 4 * 32 + 7 + 13);
        let n = board.led_count();
        let mut s = Session::new(board, n);
        let ops = s.step(&vec![Color::BLACK; n]);
        let mut want: Vec<Op> = (0x20..=0x27).map(|r| Op::Feature(cc(r, 0, 0))).collect();
        want.extend((0x90..=0x92).map(|r| Op::Feature(cc(r, 0, 0))));
        want.push(Op::Feature(cc(0x28, 0xFF, 0x07)));
        want.push(Op::Feature(cc(0x31, 0, 0)));
        want.push(Op::Feature(cc(0x32, 0x7B, 0)));
        want.push(Op::Sleep(50));
        assert_eq!(ops[..want.len()], want[..]);
        // LED10 now a 13-LED onboard strip on header 0x64, which needs its own effect bit
        assert!(ops.contains(&Op::Feature(cc(0x32, 0x7B, 0))));
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::Feature(p) if p[1] == 0x64))
        );
    }

    #[test]
    fn device_positions_cover_every_led() {
        let mut zones = IT5711.to_vec();
        apply_it5711_counts(&mut zones, 0x00, 0);
        let d = device("x".into(), "y", &zones);
        let total: usize = zones.iter().map(zone_leds).sum();
        assert_eq!(d.leds.len(), total);
        assert!(d.leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn a_set_length_sizes_a_header() {
        let mut zones = IT5711.to_vec();
        apply_it5711_counts(&mut zones, 0x03, 0);
        crate::lengths::set_one("hid:fusion2-test", 1, 100);
        crate::lengths::set_one("hid:fusion2-test", 3, 900);
        crate::lengths::set_one("hid:fusion2-test", 10, 2);
        sized("hid:fusion2-test", &mut zones);
        let leds: Vec<usize> = zones.iter().map(zone_leds).collect();
        assert_eq!(leds[..4], [32, 100, 32, 256]);
        assert_eq!(leds[10], 13);
        let names: Vec<String> = crate::lengths::resizable()
            .into_iter()
            .filter(|z| z.device == "hid:fusion2-test")
            .map(|z| z.name)
            .collect();
        assert_eq!(names, ["Header 1", "Header 2", "Header 3", "Header 4"]);
    }
}
