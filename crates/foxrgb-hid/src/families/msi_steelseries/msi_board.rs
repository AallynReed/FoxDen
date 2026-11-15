//! Ported from OpenRGB Controllers/MSIMotherboardController (GPL-2.0-or-later).

use super::common::{Driver, Op, Proto, device, get_feature, rows};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

const J_RGB_1: u8 = 1;
const J_RGB_2: u8 = 2;
const J_PIPE_1: u8 = 3;
const J_PIPE_2: u8 = 4;
const J_RAINBOW_1: u8 = 8;
const J_RAINBOW_2: u8 = 9;
const J_RAINBOW_3: u8 = 10;
const J_CORSAIR: u8 = 11;
const ON_BOARD_LED_0: u8 = 13;

const PID_COMMON: u16 = 0x0076;
/// LEDs given to each addressable header; the sync frame holds 40 per JRAINBOW.
const HEADER_LEDS: usize = 40;
const SYNC_JRAINBOW: u8 = 40;
const SYNC_CORSAIR: u8 = 120;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direct {
    Disabled,
    PerLed,
    ZoneBased,
}

struct Config185 {
    pid: u16,
    onboard: usize,
    pipe1: usize,
    pipe2: usize,
    jrgbs: usize,
    zones: &'static [u8],
    mode: Direct,
    min_fw: u8,
}

const fn board(name: &'static str, vid: u16, pid: u16, page: u16, usage: u16) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: None,
        usage_page: Some(page),
        usage: Some(usage),
        open,
    }
}

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

/// ZoneData: effect, colour, speed/brightness flags, colour 2, colour flags, padding.
fn put_color(p: &mut [u8], at: usize, c: Color) {
    p[at + 1..at + 4].copy_from_slice(&[c.0, c.1, c.2]);
    p[at + 5..at + 8].copy_from_slice(&[c.0, c.1, c.2]);
}

// ---- 112-byte generation ----

fn off_112(zone: u8) -> Option<usize> {
    match zone {
        J_RGB_1 => Some(1),
        J_RAINBOW_1 => Some(11),
        J_CORSAIR => Some(21),
        13..=19 => Some(41 + 10 * (zone - ON_BOARD_LED_0) as usize),
        _ => None,
    }
}

const ZONES_112: &[(u8, usize)] = &[
    (J_RGB_1, 1),
    (J_RAINBOW_1, 1),
    (J_CORSAIR, 1),
    (ON_BOARD_LED_0, 7),
];

/// `zone_based_per_led_data` as MSIMotherboard112Controller's constructor leaves it.
fn per_led_112() -> [u8; 112] {
    let mut p = [0u8; 112];
    p[0] = 0x52;
    for at in (1..111).step_by(10) {
        p[at] = 0x01;
    }
    for at in [1, 11, 41, 51, 61, 71, 81, 91, 101] {
        p[at + 4] = 0x28;
        p[at + 8] |= 0x80;
    }
    p
}

/// Direct frame: SetLedColor for every LED, zones in order and LEDs last to first, as DeviceUpdateLEDs does.
fn frame_112(colors: &[Color]) -> [u8; 112] {
    let mut p = per_led_112();
    let mut base = 0;
    for &(zone, count) in ZONES_112 {
        for i in (0..count).rev() {
            let mut z = zone + i as u8;
            if z >= ON_BOARD_LED_0 {
                z += 1;
            }
            if let Some(at) = off_112(z) {
                put_color(
                    &mut p,
                    at,
                    colors.get(base + i).copied().unwrap_or(Color::BLACK),
                );
            }
        }
        base += count;
    }
    p
}

struct Gen112;

impl Proto for Gen112 {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![Op::Feature(frame_112(colors).to_vec())]
    }
}

// ---- 162-byte generation ----

fn off_162(zone: u8) -> Option<usize> {
    match zone {
        J_RGB_1 => Some(1),
        J_RGB_2 => Some(151),
        J_RAINBOW_1 => Some(11),
        J_RAINBOW_2 => Some(141),
        J_CORSAIR => Some(21),
        13..=23 => Some(41 + 10 * (zone - ON_BOARD_LED_0) as usize),
        _ => None,
    }
}

fn zones_162(pid: u16) -> (usize, &'static [u8]) {
    CONFIGS_162
        .iter()
        .find(|c| c.0 == pid)
        .map(|c| (c.1, c.2))
        .unwrap_or((10, S162_ZONES_SET0))
}

fn led_counts(zones: &[u8], onboard: usize) -> Vec<(u8, usize)> {
    zones
        .iter()
        .map(|&z| (z, if z == ON_BOARD_LED_0 { onboard } else { 1 }))
        .collect()
}

/// SetMode(STATIC, speed 0, brightness 10, custom colour) then SetZoneColor, as "Direct" does on this generation.
fn apply_162(data: &mut [u8], zones: &[(u8, usize)], colors: &[Color]) {
    let mut base = 0;
    for &(zone, count) in zones {
        for i in (0..count).rev() {
            let z = zone + i as u8;
            let c = colors.get(base + i).copied().unwrap_or(Color::BLACK);
            let Some(at) = off_162(z) else { continue };
            if z <= ON_BOARD_LED_0 {
                data[at] = 0x01;
                data[at + 4] = 0x28;
                data[at + 8] |= 0x80;
                data[at + 9] = 0;
                data[at + 4] &= !0x80;
                data[at + 8] &= !0x01;
            }
            let next = if z >= ON_BOARD_LED_0 {
                off_162(z + 1)
            } else {
                None
            };
            if let Some(b) = next {
                data[b] = 0x01;
                data[b + 4] = 0x28;
                data[b + 8] |= 0x80;
                data[b + 9] = 0;
            }
            if z <= ON_BOARD_LED_0 {
                put_color(data, at, c);
            }
            if let Some(b) = next {
                put_color(data, b, c);
            }
        }
        base += count;
    }
    data[161] = 0;
}

/// The settings read at open, with the constructor's save flag and on-board sync master set.
fn settings_162(read: &[u8]) -> [u8; 162] {
    let mut d = [0u8; 162];
    let n = read.len().min(162);
    d[..n].copy_from_slice(&read[..n]);
    d[0] = 0x52;
    d[161] = 0;
    d[41 + 8] = 0x81;
    d
}

struct Gen162 {
    data: [u8; 162],
    zones: Vec<(u8, usize)>,
}

impl Proto for Gen162 {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        apply_162(&mut self.data, &self.zones, colors);
        vec![Op::Feature(self.data.to_vec())]
    }
}

// ---- 185-byte generation ----

fn config_185(pid: u16) -> Option<&'static Config185> {
    CONFIGS_185.iter().find(|c| c.pid == pid)
}

fn off_185(zone: u8, mixed: bool) -> Option<usize> {
    Some(match zone {
        J_RGB_1 => 1,
        J_RGB_2 if mixed => 134,
        J_RGB_2 => 174,
        J_RAINBOW_1 => 31,
        J_RAINBOW_2 => 42,
        J_RAINBOW_3 | J_CORSAIR => 53,
        J_PIPE_1 => 11,
        J_PIPE_2 => 21,
        19 if mixed => 64,
        13..=22 => 74 + 10 * (zone - ON_BOARD_LED_0) as usize,
        12 => 64,
        _ => return None,
    })
}

/// Default member values of FeaturePacket_185.
fn defaults_185() -> [u8; 185] {
    let mut p = [0u8; 185];
    p[0] = 0x52;
    for at in [
        1, 11, 21, 31, 42, 53, 64, 74, 84, 94, 104, 114, 124, 134, 144, 154, 164, 174,
    ] {
        p[at] = 0x01;
    }
    p[41] = 100;
    p[52] = 100;
    p[57] = 40;
    p
}

/// enable_per_led_msg after the constructor and SelectPerLedProtocol.
fn enable_185(sync: bool, r1: bool, r2: bool, corsair: bool) -> [u8; 185] {
    let mut p = defaults_185();
    let mut zone = |at: usize, flags: u8| {
        p[at + 4] = flags;
        p[at + 8] = 0x80;
    };
    zone(1, 0x08);
    zone(11, 0x2A);
    zone(21, 0x2A);
    zone(31, 0x29);
    zone(42, 0x29);
    zone(64, 0x28);
    for at in (84..=164).step_by(10) {
        zone(at, 0x28);
    }
    zone(174, 0x2A);
    p[41] = 0x28;
    p[52] = 0x28;
    p[57] = 0x29;
    p[58] = 0x00;
    p[61] = 0x82;
    p[63] = 0x78;
    p[74] = 0x25;
    p[78] = 0x29 | 0x80;
    p[82] = 0xBF;
    if sync {
        p[31] = 0x01;
        p[41] = SYNC_JRAINBOW;
        p[42] = 0x01;
        p[52] = SYNC_JRAINBOW;
        p[53] = 0x01;
        p[63] = SYNC_CORSAIR;
        p[82] = 0xBF;
    } else {
        if r1 {
            p[31] = 0x25;
            p[41] = 200;
        }
        if r2 {
            p[42] = 0x25;
            p[52] = 240;
        }
        if corsair {
            p[53] = 0x25;
            p[63] = 240;
        }
        p[82] = 0xB1;
    }
    p
}

/// `zone_based_per_led_data` as the constructor leaves it.
fn zone_based_185() -> [u8; 185] {
    let mut p = defaults_185();
    for at in [
        1, 11, 21, 31, 42, 74, 84, 94, 104, 114, 124, 134, 144, 154, 164, 174,
    ] {
        p[at + 4] = 0x28;
        p[at + 8] |= 0x80;
    }
    p
}

fn per_led_header(hdr1: u8, hdr2: u8) -> [u8; 725] {
    let mut p = [0u8; 725];
    p[..5].copy_from_slice(&[0x53, 0x25, hdr1, hdr2, 0x00]);
    p
}

impl Config185 {
    fn has(&self, zone: u8) -> bool {
        self.zones.contains(&zone)
    }

    fn mixed(&self) -> bool {
        self.pid >= 0x7D03
    }

    fn count(&self, zone: u8) -> usize {
        match zone {
            J_RGB_1 | J_RGB_2 => 1,
            J_PIPE_1 => self.pipe1,
            J_PIPE_2 => self.pipe2,
            ON_BOARD_LED_0 => self.onboard,
            _ if self.mode == Direct::ZoneBased => 1,
            _ => HEADER_LEDS,
        }
    }

    fn leds(&self) -> Vec<(u8, usize)> {
        self.zones.iter().map(|&z| (z, self.count(z))).collect()
    }

    /// Whether anything is driven per LED; Update sends nothing otherwise.
    fn per_led_active(&self) -> bool {
        [
            ON_BOARD_LED_0,
            J_RAINBOW_1,
            J_RAINBOW_2,
            J_CORSAIR,
            J_RAINBOW_3,
        ]
        .iter()
        .any(|z| self.has(*z))
    }

    /// sync_direct_mode from the settings, with each supported header set to its LED count.
    fn sync(&self, settings: &[u8]) -> bool {
        let at = |i: usize| settings.get(i).copied().unwrap_or(0);
        let mut r1 = at(41);
        let mut r2 = at(52);
        let mut r3 = at(63);
        if self.has(J_RAINBOW_1) {
            r1 = self.count(J_RAINBOW_1) as u8;
        }
        if self.has(J_RAINBOW_2) {
            r2 = self.count(J_RAINBOW_2) as u8;
        }
        if self.has(J_RAINBOW_3) {
            r3 = self.count(J_RAINBOW_3) as u8;
        }
        !(r1 > SYNC_JRAINBOW || r2 > SYNC_JRAINBOW || r3 > SYNC_CORSAIR)
    }

    fn enable(&self, sync: bool) -> [u8; 185] {
        let corsair = self.has(J_CORSAIR) || self.has(J_RAINBOW_3);
        enable_185(sync, self.has(J_RAINBOW_1), self.has(J_RAINBOW_2), corsair)
    }

    /// SetLedColor in per-LED mode, then Update: the packets in the order they are sent.
    fn per_led_frames(&self, sync: bool, colors: &[Color]) -> Vec<Op> {
        let mut main = per_led_header(0x06, 0x00);
        let mut r1 = per_led_header(0x04, 0x00);
        let mut r2 = per_led_header(0x04, 0x01);
        let mut cors = per_led_header(0x05, 0x00);
        let pipe1 = self.onboard;
        let pipe2 = pipe1 + self.pipe1;
        let jrgb = pipe2 + self.pipe2;
        let r1_sync = jrgb + self.jrgbs;
        let r2_sync = r1_sync + SYNC_JRAINBOW as usize;
        let cors_sync = r2_sync + SYNC_JRAINBOW as usize;
        let mut base = 0;
        for (zone, count) in self.leds() {
            for i in (0..count).rev() {
                let c = colors.get(base + i).copied().unwrap_or(Color::BLACK);
                let (packet, start, max) = match zone {
                    J_RAINBOW_1 if sync => (&mut main, r1_sync, SYNC_JRAINBOW as usize),
                    J_RAINBOW_1 => (&mut r1, 0, 200),
                    J_RAINBOW_2 if sync => (&mut main, r2_sync, SYNC_JRAINBOW as usize),
                    J_RAINBOW_2 => (&mut r2, 0, 240),
                    J_RAINBOW_3 | J_CORSAIR if sync => {
                        (&mut main, cors_sync, SYNC_CORSAIR as usize)
                    }
                    J_RAINBOW_3 | J_CORSAIR => (&mut cors, 0, 240),
                    ON_BOARD_LED_0 => (&mut main, 0, self.onboard),
                    J_RGB_1 => (&mut main, jrgb, 1),
                    J_RGB_2 => (&mut main, jrgb + 1, 1),
                    J_PIPE_1 => (&mut main, pipe1, self.pipe1),
                    J_PIPE_2 => (&mut main, pipe2, self.pipe2),
                    _ => continue,
                };
                let slot = 5 + (start + i) * 3;
                if i < max && slot + 3 <= packet.len() {
                    packet[slot..slot + 3].copy_from_slice(&[c.0, c.1, c.2]);
                }
            }
            base += count;
        }
        let mut ops = Vec::new();
        if !sync {
            if self.has(J_RAINBOW_1) {
                ops.extend([Op::Feature(r1.to_vec()), Op::Sleep(13)]);
            }
            if self.has(J_RAINBOW_2) {
                ops.extend([Op::Feature(r2.to_vec()), Op::Sleep(13)]);
            }
            if self.has(J_CORSAIR) || self.has(J_RAINBOW_3) {
                ops.extend([Op::Feature(cors.to_vec()), Op::Sleep(13)]);
            }
        }
        ops.push(Op::Feature(main.to_vec()));
        ops
    }

    /// SetLedColor in zone-based mode, then Update.
    fn zone_based_frame(&self, colors: &[Color]) -> [u8; 185] {
        let mut p = zone_based_185();
        let mut base = 0;
        for (zone, count) in self.leds() {
            for i in (0..count).rev() {
                let c = colors.get(base + i).copied().unwrap_or(Color::BLACK);
                if matches!(zone, J_RAINBOW_1 | J_RAINBOW_2 | J_PIPE_1 | J_PIPE_2) && i != 0 {
                    continue;
                }
                let z = if zone >= ON_BOARD_LED_0 {
                    zone + i as u8 + 1
                } else {
                    zone
                };
                if let Some(at) = off_185(z, self.mixed()) {
                    put_color(&mut p, at, c);
                }
            }
            base += count;
        }
        p
    }
}

struct Gen185 {
    config: &'static Config185,
    sync: bool,
    enabled: bool,
}

impl Proto for Gen185 {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        match self.config.mode {
            Direct::ZoneBased => vec![Op::Feature(self.config.zone_based_frame(colors).to_vec())],
            _ => {
                let mut ops = Vec::new();
                if !self.enabled {
                    ops.push(Op::Feature(self.config.enable(self.sync).to_vec()));
                    self.enabled = true;
                }
                ops.extend(self.config.per_led_frames(self.sync, colors));
                ops
            }
        }
    }

    fn reset(&mut self) {
        self.enabled = false;
    }
}

/// APROM version from ReadFwVersion.
fn read_fw(hid: &HidDevice) -> u8 {
    let mut req = [0xCCu8; 64];
    req[0] = 0x01;
    req[1] = 0xB0;
    let mut resp = [0u8; 64];
    let _ = hid.write(&req);
    let _ = hid.read_timeout(&mut resp, 1000);
    let aprom = resp[2];
    req[1] = 0xB6;
    let _ = hid.write(&req);
    let _ = hid.read_timeout(&mut resp, 1000);
    aprom
}

/// The board id a common-PID board reports in the first four characters of its serial.
fn common_pid(serial: &str) -> Option<u16> {
    u16::from_str_radix(serial.get(..4)?, 16).ok()
}

#[derive(Debug, PartialEq, Eq)]
enum Generation {
    G185,
    G162,
    G112,
    G761,
}

/// DetectMSIMotherboardControllers picks the generation by the length of settings report 0x52.
fn generation(len: usize) -> Generation {
    match len {
        185 | 186 => Generation::G185,
        162 | 163 => Generation::G162,
        112 | 113 => Generation::G112,
        _ => Generation::G761,
    }
}

fn layout(zones: &[(u8, usize)]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let counts: Vec<usize> = zones.iter().map(|z| z.1).filter(|&n| n > 0).collect();
    rows(&counts)
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let settings = get_feature(&hid, 0x52, 200)?;
    let id = info.device_id("msi-mystic");
    match generation(settings.len()) {
        Generation::G185 => {
            let fw = read_fw(&hid);
            let pid = if info.pid == PID_COMMON {
                common_pid(&info.serial).ok_or("serial does not carry the board id")?
            } else {
                info.pid
            };
            let config = config_185(pid).ok_or("board has no Mystic Light direct mode")?;
            if fw < config.min_fw {
                return Err("firmware below the minimum OpenRGB supports".into());
            }
            if config.mode == Direct::Disabled
                || (config.mode == Direct::PerLed && !config.per_led_active())
            {
                return Err("board has no Mystic Light direct mode".into());
            }
            let dev = device(
                id,
                det.name,
                DeviceKind::Motherboard,
                layout(&config.leds()),
            );
            let proto = Gen185 {
                config,
                sync: config.sync(&settings),
                enabled: false,
            };
            Ok(Driver::boxed(hid, "msi mystic light", dev, proto))
        }
        Generation::G162 => {
            let (onboard, zones) = zones_162(info.pid);
            let zones = led_counts(zones, onboard);
            let dev = device(id, det.name, DeviceKind::Motherboard, layout(&zones));
            let proto = Gen162 {
                data: settings_162(&settings),
                zones,
            };
            Ok(Driver::boxed(hid, "msi mystic light", dev, proto))
        }
        Generation::G112 => {
            let dev = device(id, det.name, DeviceKind::Motherboard, layout(ZONES_112));
            Ok(Driver::boxed(hid, "msi mystic light", dev, Gen112))
        }
        Generation::G761 => super::msi_board761::open(hid, info, det),
    }
}

const ZONES_SET0: &[u8] = &[J_RGB_1, J_RGB_2, J_RAINBOW_1, J_RAINBOW_2, ON_BOARD_LED_0];
const ZONES_SET1: &[u8] = &[J_RGB_1, J_RAINBOW_1, J_RAINBOW_2, J_CORSAIR];
const ZONES_SET2: &[u8] = &[J_RGB_1, J_RAINBOW_1, J_RAINBOW_2, ON_BOARD_LED_0];
const ZONES_SET3: &[u8] = &[
    J_RGB_1,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_CORSAIR,
    J_PIPE_1,
    ON_BOARD_LED_0,
];
const ZONES_SET5: &[u8] = &[J_RGB_1, J_RAINBOW_1];
const ZONES_SET6: &[u8] = &[J_RAINBOW_1];
const ZONES_SET7: &[u8] = &[
    J_RGB_1,
    J_RGB_2,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_PIPE_1,
    ON_BOARD_LED_0,
];
const ZONES_SET8: &[u8] = &[
    J_RGB_1,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_CORSAIR,
    J_PIPE_1,
    J_PIPE_2,
];
const ZONES_SET9: &[u8] = &[J_RGB_1, J_RAINBOW_1, J_RAINBOW_2, J_CORSAIR, J_PIPE_1];
const ZONES_SET10: &[u8] = &[J_RGB_1, J_RAINBOW_1, J_RAINBOW_2, J_RAINBOW_3];
const ZONES_SET11: &[u8] = &[J_RGB_1, J_RAINBOW_1, J_RAINBOW_2];
const ZONES_SET12: &[u8] = &[
    J_RGB_1,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_RAINBOW_3,
    J_PIPE_1,
    ON_BOARD_LED_0,
];
const ZONES_SET13: &[u8] = &[J_RGB_1, J_RGB_2, J_RAINBOW_1, J_RAINBOW_2];
const ZONES_SET14: &[u8] = &[J_RAINBOW_1, J_RAINBOW_2];
const ZONES_SET15: &[u8] = &[
    J_RGB_1,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_RAINBOW_3,
    J_PIPE_1,
    J_PIPE_2,
    ON_BOARD_LED_0,
];
const ZONES_SET16: &[u8] = &[
    J_RGB_1,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_RAINBOW_3,
    J_PIPE_1,
    J_PIPE_2,
];
const ZONES_SET17: &[u8] = &[J_RGB_1, J_RGB_2, J_RAINBOW_1, J_RAINBOW_2, J_PIPE_1];
const ZONES_SET18: &[u8] = &[J_RGB_1, J_RAINBOW_1, J_RAINBOW_2, J_RAINBOW_3, J_PIPE_1];
const ZONES_SET19: &[u8] = &[J_RAINBOW_1, J_RAINBOW_2, J_RAINBOW_3];
const ZONES_SET20: &[u8] = &[J_RGB_1];
const ZONES_SET21: &[u8] = &[
    J_RGB_1,
    J_RGB_2,
    J_RAINBOW_1,
    J_RAINBOW_2,
    J_CORSAIR,
    J_PIPE_1,
    J_PIPE_2,
    ON_BOARD_LED_0,
];
const CONFIGS_185: &[Config185] = &[
    Config185 {
        pid: 0x7B93,
        onboard: 6,
        pipe1: 1,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C34,
        onboard: 0,
        pipe1: 1,
        pipe2: 1,
        jrgbs: 1,
        zones: ZONES_SET8,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C35,
        onboard: 0,
        pipe1: 1,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET9,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C36,
        onboard: 6,
        pipe1: 1,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C37,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C56,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET2,
        mode: Direct::PerLed,
        min_fw: 6,
    },
    Config185 {
        pid: 0x7C59,
        onboard: 0,
        pipe1: 8,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET9,
        mode: Direct::Disabled,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C60,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET1,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C67,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C71,
        onboard: 6,
        pipe1: 6,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C73,
        onboard: 6,
        pipe1: 4,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C75,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET2,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C76,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::Disabled,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C77,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 0,
        zones: ZONES_SET14,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C79,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C80,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C02,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C81,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C82,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C83,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::Disabled,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C84,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C86,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::Disabled,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C87,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C90,
        onboard: 6,
        pipe1: 4,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C91,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C92,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 0,
        zones: ZONES_SET6,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C94,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C95,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7C98,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D03,
        onboard: 0,
        pipe1: 15,
        pipe2: 18,
        jrgbs: 1,
        zones: ZONES_SET8,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D06,
        onboard: 4,
        pipe1: 4,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D07,
        onboard: 4,
        pipe1: 5,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET7,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D08,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D09,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D13,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET1,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D14,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D15,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D17,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D18,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D19,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D20,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D25,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D27,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET1,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D28,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET1,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D29,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 0,
        zones: ZONES_SET6,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D30,
        onboard: 6,
        pipe1: 6,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D31,
        onboard: 4,
        pipe1: 8,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET12,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D32,
        onboard: 1,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET10,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D33,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D36,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D37,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D38,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET1,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D40,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET5,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D41,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D42,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D43,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D46,
        onboard: 0,
        pipe1: 1,
        pipe2: 1,
        jrgbs: 0,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D50,
        onboard: 6,
        pipe1: 12,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D51,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET1,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D52,
        onboard: 6,
        pipe1: 14,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET3,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D53,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D54,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET0,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D59,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D67,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D69,
        onboard: 9,
        pipe1: 2,
        pipe2: 4,
        jrgbs: 1,
        zones: ZONES_SET15,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D70,
        onboard: 0,
        pipe1: 6,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET9,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D73,
        onboard: 1,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 0,
        zones: ZONES_SET6,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D74,
        onboard: 0,
        pipe1: 6,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET18,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D75,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D76,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D77,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D78,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D86,
        onboard: 0,
        pipe1: 18,
        pipe2: 4,
        jrgbs: 1,
        zones: ZONES_SET16,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D88,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET20,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D89,
        onboard: 0,
        pipe1: 6,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET18,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D90,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D91,
        onboard: 1,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET10,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D93,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET2,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D96,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D97,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D98,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7D99,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E01,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 1,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E03,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 0,
        zones: ZONES_SET6,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E06,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET11,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E07,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET10,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E09,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 0,
        zones: ZONES_SET19,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E10,
        onboard: 0,
        pipe1: 6,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET17,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E12,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x0076,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E16,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E24,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E26,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E27,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E28,
        onboard: 6,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET21,
        mode: Direct::ZoneBased,
        min_fw: 0,
    },
    Config185 {
        pid: 0x7E30,
        onboard: 0,
        pipe1: 0,
        pipe2: 0,
        jrgbs: 2,
        zones: ZONES_SET13,
        mode: Direct::PerLed,
        min_fw: 0,
    },
];
const S162_ZONES_SET0: &[u8] = &[J_RGB_1, J_RGB_2, J_RAINBOW_1, J_CORSAIR, ON_BOARD_LED_0];
const S162_ZONES_SET1: &[u8] = &[J_RGB_1, J_RGB_2, J_RAINBOW_1, ON_BOARD_LED_0];
const S162_ZONES_SET2: &[u8] = &[J_RGB_1, J_RGB_2, J_RAINBOW_1, J_RAINBOW_2, ON_BOARD_LED_0];
const S162_ZONES_SET3: &[u8] = &[J_RAINBOW_1, ON_BOARD_LED_0];
const CONFIGS_162: &[(u16, usize, &[u8])] = &[
    (0x1720, 10, S162_ZONES_SET0),
    (0x7B12, 10, S162_ZONES_SET0),
    (0x7B17, 10, S162_ZONES_SET0),
    (0x7B18, 6, S162_ZONES_SET1),
    (0x7B50, 6, S162_ZONES_SET2),
    (0x7B85, 7, S162_ZONES_SET0),
    (0x7B92, 10, S162_ZONES_SET0),
    (0xB926, 3, S162_ZONES_SET3),
];
static DETECTORS: &[Detector] = &[
    board("MSI Mystic Light MS_1720", 0x1462, 0x1720, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B12", 0x1462, 0x7B12, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B16", 0x1462, 0x7B16, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B17", 0x1462, 0x7B17, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B18", 0x1462, 0x7B18, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B50", 0x1462, 0x7B50, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B85", 0x1462, 0x7B85, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B92", 0x1462, 0x7B92, 0x0001, 0x00),
    board("MSI Mystic Light MS_7B93", 0x1462, 0x7B93, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C02", 0x1462, 0x7C02, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C34", 0x1462, 0x7C34, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C35", 0x1462, 0x7C35, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C36", 0x1462, 0x7C36, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C37", 0x1462, 0x7C37, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C56", 0x1462, 0x7C56, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C59", 0x1462, 0x7C59, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C60", 0x1462, 0x7C60, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C67", 0x1462, 0x7C67, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C71", 0x1462, 0x7C71, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C73", 0x1462, 0x7C73, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C75", 0x1462, 0x7C75, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C76", 0x1462, 0x7C76, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C77", 0x1462, 0x7C77, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C79", 0x1462, 0x7C79, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C80", 0x1462, 0x7C80, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C81", 0x1462, 0x7C81, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C82", 0x1462, 0x7C82, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C83", 0x1462, 0x7C83, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C84", 0x1462, 0x7C84, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C86", 0x1462, 0x7C86, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C87", 0x1462, 0x7C87, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C90", 0x1462, 0x7C90, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C91", 0x1462, 0x7C91, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C92", 0x1462, 0x7C92, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C94", 0x1462, 0x7C94, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C95", 0x1462, 0x7C95, 0x0001, 0x00),
    board("MSI Mystic Light MS_7C98", 0x1462, 0x7C98, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D03", 0x1462, 0x7D03, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D04", 0x1462, 0x7D04, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D06", 0x1462, 0x7D06, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D07", 0x1462, 0x7D07, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D08", 0x1462, 0x7D08, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D09", 0x1462, 0x7D09, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D13", 0x1462, 0x7D13, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D14", 0x1462, 0x7D14, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D15", 0x1462, 0x7D15, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D17", 0x1462, 0x7D17, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D18", 0x1462, 0x7D18, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D19", 0x1462, 0x7D19, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D20", 0x1462, 0x7D20, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D25", 0x1462, 0x7D25, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D27", 0x1462, 0x7D27, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D28", 0x1462, 0x7D28, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D29", 0x1462, 0x7D29, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D30", 0x1462, 0x7D30, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D31", 0x1462, 0x7D31, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D32", 0x1462, 0x7D32, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D33", 0x1462, 0x7D33, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D36", 0x1462, 0x7D36, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D37", 0x1462, 0x7D37, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D38", 0x1462, 0x7D38, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D40", 0x1462, 0x7D40, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D41", 0x1462, 0x7D41, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D42", 0x1462, 0x7D42, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D43", 0x1462, 0x7D43, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D46", 0x1462, 0x7D46, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D50", 0x1462, 0x7D50, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D51", 0x1462, 0x7D51, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D52", 0x1462, 0x7D52, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D53", 0x1462, 0x7D53, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D54", 0x1462, 0x7D54, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D59", 0x1462, 0x7D59, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D67", 0x1462, 0x7D67, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D69", 0x1462, 0x7D69, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D70", 0x1462, 0x7D70, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D73", 0x1462, 0x7D73, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D74", 0x1462, 0x7D74, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D75", 0x1462, 0x7D75, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D76", 0x1462, 0x7D76, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D77", 0x1462, 0x7D77, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D78", 0x1462, 0x7D78, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D86", 0x1462, 0x7D86, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D88", 0x1462, 0x7D88, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D89", 0x1462, 0x7D89, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D90", 0x1462, 0x7D90, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D91", 0x1462, 0x7D91, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D93", 0x1462, 0x7D93, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D96", 0x1462, 0x7D96, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D97", 0x1462, 0x7D97, 0x0001, 0x00),
    board("MSI Mystic Light MS_7D98", 0x1462, 0x7D98, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E01", 0x1462, 0x7E01, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E03", 0x1462, 0x7E03, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E06", 0x1462, 0x7E06, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E07", 0x1462, 0x7E07, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E09", 0x1462, 0x7E09, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E10", 0x1462, 0x7E10, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E20", 0x1462, 0x7E20, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E30", 0x1462, 0x7E30, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E32", 0x1462, 0x7E32, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E34", 0x1462, 0x7E34, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E59", 0x1462, 0x7E59, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E66", 0x1462, 0x7E66, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E70", 0x1462, 0x7E70, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E76", 0x1462, 0x7E76, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E80", 0x1462, 0x7E80, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E81", 0x1462, 0x7E81, 0x0001, 0x00),
    board("MSI Mystic Light MS_7E86", 0x1462, 0x7E86, 0x0001, 0x00),
    board("MSI Mystic Light MS_B926", 0x1462, 0xB926, 0x0001, 0x00),
    board("MSI Mystic Light Common", 0x0DB0, 0x0076, 0x0001, 0x00),
    board("MSI Mystic Light X870", 0x0DB0, 0x0076, 0xFF00, 0x01),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(op: &Op) -> &[u8] {
        match op {
            Op::Feature(b) => b,
            other => panic!("expected a feature report, got {other:?}"),
        }
    }

    // MSIMotherboardControllerDetect.cpp: lines outside ENABLE_UNTESTED_MYSTIC_LIGHT, 7D99 left to msi.rs.
    #[test]
    fn detectors_match_openrgb_list() {
        assert_eq!(DETECTORS.len(), 110);
        assert!(DETECTORS.iter().all(|d| d.pid != 0x7D99));
        assert!(DETECTORS.iter().all(|d| d.interface.is_none()));
        let x870 = DETECTORS
            .iter()
            .find(|d| d.name == "MSI Mystic Light X870")
            .unwrap();
        assert_eq!(
            (x870.vid, x870.pid, x870.usage_page, x870.usage),
            (0x0DB0, 0x0076, Some(0xFF00), Some(1))
        );
        let first = &DETECTORS[0];
        assert_eq!(
            (first.vid, first.pid, first.usage_page, first.usage),
            (0x1462, 0x1720, Some(1), Some(0))
        );
        for untested in [0x3EA4, 0x7B10, 0x905D, 0x7C42] {
            assert!(DETECTORS.iter().all(|d| d.pid != untested));
        }
    }

    // DetectMSIMotherboardControllers length checks.
    #[test]
    fn generation_by_length() {
        assert_eq!(generation(185), Generation::G185);
        assert_eq!(generation(186), Generation::G185);
        assert_eq!(generation(162), Generation::G162);
        assert_eq!(generation(163), Generation::G162);
        assert_eq!(generation(112), Generation::G112);
        assert_eq!(generation(113), Generation::G112);
        assert_eq!(generation(184), Generation::G761);
        assert_eq!(generation(0), Generation::G761);
    }

    // MSIMotherboard185Controller constructor: pid = stoi(serial.substr(0, 4), 16).
    #[test]
    fn common_pid_from_serial() {
        assert_eq!(common_pid("7E12A0000000"), Some(0x7E12));
        assert_eq!(common_pid("7d"), None);
    }

    // enable_per_led_msg + SelectPerLedProtocol (sync) equals what msi.rs already sends to the 7D99.
    #[test]
    fn sync_enable_matches_msi_rs() {
        assert_eq!(
            enable_185(true, true, true, false),
            crate::msi::enable_direct()
        );
        assert_eq!(
            enable_185(true, false, false, true),
            crate::msi::enable_direct()
        );
    }

    // SelectPerLedProtocol, non-sync branch.
    #[test]
    fn non_sync_enable() {
        let p = enable_185(false, true, false, true);
        assert_eq!((p[31], p[41]), (0x25, 200));
        assert_eq!((p[42], p[52]), (0x01, 0x28));
        assert_eq!((p[53], p[63]), (0x25, 240));
        assert_eq!(p[82], 0xB1);
        assert_eq!(p[78], 0xA9);
        assert_eq!(p[184], 0);
    }

    // FeaturePacket_PerLED_185 slot layout from the constructor, 7D99 row: same bytes as msi.rs.
    #[test]
    fn sync_frame_matches_msi_rs_for_7d99() {
        let cfg = config_185(0x7D99).unwrap();
        let n: usize = cfg.leds().iter().map(|z| z.1).sum();
        // OpenRGB's 7D99 row has no ONBOARD zone; msi.rs also drives its six reserved slots
        assert_eq!(n + 6, crate::msi::LED_COUNT);
        let colors: Vec<Color> = (0..n)
            .map(|i| Color(i as u8, 2 * i as u8, 255 - i as u8))
            .collect();
        let mut theirs = vec![Color::BLACK; 6];
        theirs.extend_from_slice(&colors);
        let ops = cfg.per_led_frames(true, &colors);
        assert_eq!(ops.len(), 1);
        assert_eq!(feature(&ops[0]), &crate::msi::direct_frame(&theirs)[..]);
    }

    // Gen185 with a PER_LED board: enable_per_led_msg once, then one 0x53 frame per push.
    #[test]
    fn per_led_first_push_sends_enable() {
        let config = config_185(0x7C56).unwrap();
        let mut g = Gen185 {
            config,
            sync: true,
            enabled: false,
        };
        let colors = vec![Color(1, 2, 3); 87];
        let ops = g.frame(&colors);
        assert_eq!(ops.len(), 2);
        assert_eq!(feature(&ops[0]), &crate::msi::enable_direct()[..]);
        assert_eq!(feature(&ops[1])[..5], [0x53, 0x25, 0x06, 0x00, 0x00]);
        assert_eq!(g.frame(&colors).len(), 1);
    }

    // Update() in non-sync mode: JRAINBOW1, JRAINBOW2, JCORSAIR packets with 13 ms gaps, then the main packet.
    #[test]
    fn non_sync_frames_per_header() {
        let config = config_185(0x7C90).unwrap(); // JRGB1, JR1, JR2, JCORSAIR, PIPE1(4), ONBOARD(6)
        let n: usize = config.leds().iter().map(|z| z.1).sum();
        let colors: Vec<Color> = (0..n).map(|i| Color(i as u8, 0, 1)).collect();
        let ops = config.per_led_frames(false, &colors);
        assert_eq!(ops.len(), 7);
        let r1 = feature(&ops[0]);
        assert_eq!(r1[..5], [0x53, 0x25, 0x04, 0x00, 0x00]);
        assert_eq!(r1[5..8], [1, 0, 1]);
        assert_eq!(r1[5 + 39 * 3..5 + 40 * 3], [40, 0, 1]);
        assert_eq!(ops[1], Op::Sleep(13));
        let r2 = feature(&ops[2]);
        assert_eq!(r2[..5], [0x53, 0x25, 0x04, 0x01, 0x00]);
        assert_eq!(r2[5..8], [41, 0, 1]);
        let cors = feature(&ops[4]);
        assert_eq!(cors[..5], [0x53, 0x25, 0x05, 0x00, 0x00]);
        assert_eq!(cors[5..8], [81, 0, 1]);
        let main = feature(&ops[6]);
        assert_eq!(main[..5], [0x53, 0x25, 0x06, 0x00, 0x00]);
        // onboard 0..6, pipe1 6..10, jrgb at 10
        assert_eq!(main[5..8], [125, 0, 1]);
        assert_eq!(main[5 + 6 * 3..5 + 7 * 3], [121, 0, 1]);
        assert_eq!(main[5 + 10 * 3..5 + 11 * 3], [0, 0, 1]);
        assert!(main[5 + 11 * 3..].iter().all(|&b| b == 0));
    }

    // sync_direct_mode: a header the board lacks still counts with the length stored on the board.
    #[test]
    fn sync_follows_stored_lengths() {
        let config = config_185(0x7C67).unwrap(); // JRGB1, JRAINBOW1 only
        let mut s = defaults_185().to_vec();
        assert!(!config.sync(&s), "JRAINBOW2 stores 100 by default");
        s[52] = 40;
        assert!(config.sync(&s));
        s[63] = 121;
        assert!(!config.sync(&s));
    }

    // zone_based_per_led_data after SetLedColor for 7B93 (JRGB1, JR1, JR2, JCORSAIR, PIPE1, ONBOARD x6).
    #[test]
    fn zone_based_frame_7b93() {
        let config = config_185(0x7B93).unwrap();
        let leds = config.leds();
        assert_eq!(leds, vec![(1, 1), (8, 1), (9, 1), (11, 1), (3, 1), (13, 6)]);
        let colors: Vec<Color> = (1..=11).map(|i| Color(i, i, i)).collect();
        let p = config.zone_based_frame(&colors);
        let mut e = defaults_185();
        for at in [
            1, 11, 21, 31, 42, 74, 84, 94, 104, 114, 124, 134, 144, 154, 164, 174,
        ] {
            e[at + 4] = 0x28;
            e[at + 8] = 0x80;
        }
        let set = |e: &mut [u8; 185], at: usize, v: u8| {
            e[at + 1..at + 4].fill(v);
            e[at + 5..at + 8].fill(v);
        };
        set(&mut e, 1, 1);
        set(&mut e, 31, 2);
        set(&mut e, 42, 3);
        set(&mut e, 53, 4);
        set(&mut e, 11, 5);
        // onboard LED i goes to ON_BOARD_LED_(i+1)
        for i in 0..6 {
            set(&mut e, 84 + 10 * i, 6 + i as u8);
        }
        assert_eq!(p, e);
    }

    // GetZoneData with mixed_mode_support (pid >= 0x7D03): JRGB2 -> on_board_led_6, ON_BOARD_LED_6 -> outerll120.
    #[test]
    fn zone_based_mixed_mode_7e28() {
        let config = config_185(0x7E28).unwrap();
        assert!(config.mixed());
        let leds = config.leds();
        let n: usize = leds.iter().map(|z| z.1).sum();
        let mut colors = vec![Color::BLACK; n];
        colors[1] = Color(9, 8, 7); // JRGB2
        let last = n - 1; // onboard LED 5 -> ON_BOARD_LED_6
        colors[last] = Color(5, 5, 5);
        let p = config.zone_based_frame(&colors);
        assert_eq!(p[135..138], [9, 8, 7]);
        assert_eq!(p[65..68], [5, 5, 5]);
        assert_eq!(p[175..178], [0, 0, 0]);
    }

    // Boards OpenRGB gives no Direct mode.
    #[test]
    fn disabled_boards() {
        assert_eq!(config_185(0x7C59).unwrap().mode, Direct::Disabled);
        assert!(!config_185(0x7D88).unwrap().per_led_active());
        assert!(config_185(0x1234).is_none());
        assert_eq!(config_185(0x7C56).unwrap().min_fw, 6);
    }

    // MSIMotherboard112Controller constructor + SetLedColor (onboard LED i -> on_board_led_(i+1), LED 6 dropped).
    #[test]
    fn frame_112_bytes() {
        let colors: Vec<Color> = (1..=10).map(|i| Color(i, 0, i)).collect();
        let p = frame_112(&colors);
        let mut e = [0u8; 112];
        e[0] = 0x52;
        for at in (1..111).step_by(10) {
            e[at] = 1;
        }
        for at in [1, 11, 41, 51, 61, 71, 81, 91, 101] {
            e[at + 4] = 0x28;
            e[at + 8] = 0x80;
        }
        let set = |e: &mut [u8; 112], at: usize, v: u8| {
            e[at + 1..at + 4].copy_from_slice(&[v, 0, v]);
            e[at + 5..at + 8].copy_from_slice(&[v, 0, v]);
        };
        set(&mut e, 1, 1);
        set(&mut e, 11, 2);
        set(&mut e, 21, 3);
        for i in 0..6 {
            set(&mut e, 51 + 10 * i, 4 + i as u8);
        }
        assert_eq!(p, e);
    }

    // MSIMotherboard162Controller SetMode/SetZoneColor/Update for a 3-LED onboard board (B926).
    #[test]
    fn apply_162_b926() {
        let (onboard, zones) = zones_162(0xB926);
        let zones = led_counts(zones, onboard);
        assert_eq!(zones, vec![(J_RAINBOW_1, 1), (ON_BOARD_LED_0, 3)]);
        let mut read = vec![0x52u8; 162];
        read[161] = 1;
        let mut d = settings_162(&read);
        assert_eq!(d[49], 0x81);
        apply_162(
            &mut d,
            &zones,
            &[
                Color(1, 2, 3),
                Color(10, 11, 12),
                Color(20, 21, 22),
                Color(30, 31, 32),
            ],
        );
        let mut e = vec![0x52u8; 162];
        e[161] = 0;
        // JRAINBOW1 at 11: effect 1, flags 0x28, colour flags (0x52|0x80)&!1, padding 0
        e[11] = 1;
        e[12..15].copy_from_slice(&[1, 2, 3]);
        e[15] = 0x28;
        e[16..19].copy_from_slice(&[1, 2, 3]);
        e[19] = (0x52 | 0x80) & !1;
        e[20] = 0;
        // on_board_led at 41 from onboard LED 0, with the sync bit cleared
        e[41] = 1;
        e[42..45].copy_from_slice(&[10, 11, 12]);
        e[45] = 0x28;
        e[46..49].copy_from_slice(&[10, 11, 12]);
        e[49] = 0x80;
        e[50] = 0;
        // on_board_led_1..3 from onboard LEDs 0..2
        for (k, v) in [(51, [10, 11, 12]), (61, [20, 21, 22]), (71, [30, 31, 32])] {
            e[k] = 1;
            e[k + 1..k + 4].copy_from_slice(&v);
            e[k + 4] = 0x28;
            e[k + 5..k + 8].copy_from_slice(&v);
            e[k + 8] = 0x52 | 0x80;
            e[k + 9] = 0;
        }
        assert_eq!(d.to_vec(), e);
    }

    #[test]
    fn devices_have_one_led_per_colour() {
        let c = config_185(0x7D03).unwrap();
        let n: usize = c.leds().iter().map(|z| z.1).sum();
        assert_eq!(layout(&c.leds()).0.len(), n);
    }
}
