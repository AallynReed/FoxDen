//! Ported from OpenRGB Controllers/LogitechController/LogitechHIDPP20Controller (GPL-2.0-or-later).

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Link, on_usb_or_bluetooth, receiver, rect};
use crate::registry::{Bus, Info, Matcher};

pub const VID: u16 = 0x046D;

const fn line(interface: Option<i32>, page: u16, usage: Option<u16>, bus: Option<Bus>) -> Matcher {
    Matcher {
        name: "Logitech HID++ 2.0",
        vid: Some(VID),
        interface,
        usage_page: Some(page),
        usage,
        bus,
        open,
    }
}

/// The unified detector's `HID_PID_ANY` lines, less the Centurion one.
pub const MATCHERS: [Matcher; 5] = [
    line(None, 0xFF00, Some(2), Some(Bus::Usb)),
    line(None, 0xFF43, None, Some(Bus::Usb)),
    line(Some(1), 0xFF00, None, None),
    line(None, 0xFF43, None, Some(Bus::Bluetooth)),
    line(None, 0xFF00, Some(2), Some(Bus::Bluetooth)),
];

const SW_ID: u8 = 0x07;
const LONG: u8 = 0x11;
const DEFAULT_INDEX: u8 = 0xFF;

const FEAT_FEATURE_SET: u16 = 0x0001;
const FEAT_FIRMWARE_INFO: u16 = 0x0003;
const FEAT_DEVICE_NAME_TYPE: u16 = 0x0005;
const FEAT_PROFILE_MANAGEMENT: u16 = 0x8101;
const FEAT_ONBOARD_PROFILES: u16 = 0x8100;
const FEAT_DISABLE_KEYS_BY_USAGE: u16 = 0x4522;
const FEAT_RGB_EFFECTS: u16 = 0x8071;
const FEAT_CENTURION_RGB: u16 = 0x0600;
const FEAT_COLOR_LED_EFFECTS: u16 = 0x8070;
const FEAT_HEADSET_RGB_HOSTMODE: u16 = 0x0620;
const FEAT_PER_KEY_V1: u16 = 0x8080;
const FEAT_PER_KEY_V2: u16 = 0x8081;
const FEAT_KEYBOARD_LAYOUT: u16 = 0x4540;
const FEAT_WIRELESS_STATUS: u16 = 0x1D4B;

const FN_SET_INDIVIDUAL: u8 = 0x10;
const FN_SET_RANGE: u8 = 0x50;
const FN_SET_SINGLE_VALUE: u8 = 0x60;
const FN_FRAME_END: u8 = 0x70;

const VERY_LONG: u8 = 0x12;
const FN_8080_GET_INFO: u8 = 0x00;
const FN_8080_GET_KEY_TYPE_INFO: u8 = 0x10;
const FN_8080_GET_KEY_COLORS: u8 = 0x20;
const FN_8080_SET_KEY_COLORS: u8 = 0x30;
const FN_8080_FLUSH_LEDS: u8 = 0x50;
const KEYS_PER_FRAME_8080: usize = 14;
/// Keyboard, consumer, G-keys, buttons, logo, indicators.
const KEY_TYPES_8080: [u16; 6] = [0x0001, 0x0002, 0x0004, 0x0008, 0x0010, 0x0040];
const WINDOW_8080_MAX: usize = 4;
const WINDOW_8080_MIN: usize = 1;
const ACK_TAIL_8080_MS: u64 = 60;
const KEYTYPE40_ID12_ONLY: u16 = 0xC330;

const PLACEHOLDER: &str = "Logitech HID++ Device";
const DEVICE_TYPE_MOUSE: u8 = 3;
const IDLE_TIMEOUT_S: u16 = 60;
const SLEEP_TIMEOUT_S: u16 = 300;

const PERKEY_WINDOW_MAX: usize = 8;
const PERKEY_WINDOW_MIN: usize = 2;
const PERKEY_WINDOW_GROW_AFTER: u8 = 16;
const PERKEY_ACK_WAIT_MS: u64 = 60;
const PERKEY_ACK_TAIL_MS: u64 = 25;
const FRAME_END_BUSY_POLL_MS: u64 = 10;
const FRAME_END_BUSY_POLL_NEAR_MS: u64 = 3;
const FE_PROBE_STEP_MS: u16 = 5;
const FE_PROBE_INTERVAL: u8 = 32;
const RESYNC_KEYS_MIN: usize = 1;
const RESYNC_KEYS_MAX: usize = 4;
const RESYNC_QUIET_ZONES: usize = 8;
const REPAINT_RETRY_BACKOFF_MS: [u64; 5] = [50, 100, 200, 400, 800];
const LATE_ANSWER_GRACE_MS: u64 = 150;
const READ_DRAIN_BUDGET: usize = 64;

/// `HIDPP20RetryPolicy`: backoff before each attempt, and the read window per attempt.
#[derive(Clone, Copy)]
pub struct Policy {
    backoff: &'static [u64],
    window_ms: u64,
}

pub const RELIABLE: Policy = Policy {
    backoff: &[0, 63, 125, 250, 500, 1000, 2000],
    window_ms: 300,
};
pub const PROBE: Policy = Policy {
    backoff: &[0, 100],
    window_ms: 200,
};
pub const FIRST_CONTACT: Policy = Policy {
    backoff: &[0, 100, 250, 500],
    window_ms: 300,
};
pub const BLUETOOTH: Policy = Policy {
    backoff: &[0, 200, 500],
    window_ms: 700,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Timeout,
    Error(u8),
    Wire,
}

type Reply = Result<[u8; 60], Fail>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    pub index: u8,
    pub id: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cluster {
    pub index: u8,
    pub effects: Vec<Effect>,
}

/// `HIDPP20DeviceCapabilities`, the parts direct mode uses.
#[derive(Debug, Default)]
pub struct Caps {
    pub feature_map: BTreeMap<u16, u8>,
    pub name: String,
    pub device_type: u8,
    pub unit_id: String,
    pub serial: String,
    pub keep_onboard: bool,
    pub idx_profile_management: u8,
    pub idx_onboard_profiles: u8,
    pub idx_disable_keys: u8,
    pub idx_rgb_effects: u8,
    pub rgb_page: u16,
    pub fn_set_effect: u8,
    pub fn_sw_control: u8,
    pub fn_pwr_config: u8,
    pub fn_pwr_mode: u8,
    pub has_power_mgmt: bool,
    pub sw_control_simple: bool,
    pub clusters: Vec<Cluster>,
    pub has_zone_effects: bool,
    pub has_effect_cards: bool,
    pub template: [u8; 2],
    pub idx_perkey_v2: u8,
    pub perkey_zone_ids: Vec<u16>,
    pub has_perkey: bool,
    /// Firmware model ids: wireless, wired, third.
    pub pids: [u16; 3],
    pub idx_perkey_8080: u8,
    pub perkey_8080_max_key_count: u16,
    pub perkey_8080_types: Vec<KeyType8080>,
    pub has_perkey_8080: bool,
    pub has_numpad: bool,
    pub layout_code: u8,
}

/// `HIDPP20PerKey8080Type`: one confirmed keyType and its keyIds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyType8080 {
    pub key_type: u16,
    pub key_ids: Vec<u8>,
}

/// What `Probe` found.
#[derive(Debug, PartialEq, Eq)]
pub enum Probed {
    NoAnswer,
    /// Answered, but with a lighting feature whose path is not ported (0x0620).
    Unported(u16),
    NoLighting,
    Lit,
}

struct Packet {
    function: u8,
    payload: [u8; 16],
    len: usize,
    zones: Vec<u8>,
    acked: bool,
}

/// `PerKeyFrameResult`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FrameResult {
    pub frame_end_acked: bool,
    pub attempted: Vec<u8>,
    pub acked: Vec<u8>,
}

/// One HID++ 2.0 device on a node: `LogitechHIDPP20Controller` plus the direct-mode parts of
/// `RGBController_LogitechHIDPP20`, without the reader, power and sender threads.
pub struct Hidpp<L: Link> {
    pub link: L,
    /// `dev_perkey_vl`: the 0x12 collection; `None` sends it on `link`.
    pub vl: Option<L>,
    index: u8,
    wireless: bool,
    bluetooth: bool,
    pairing_name: String,
    pub caps: Caps,
    wire_error: Option<String>,
    claimed: bool,
    upgrade_to_5: bool,
    prep_applied: bool,
    pipelining: bool,
    pipeline: Vec<(u8, u8, Vec<u8>)>,
    packets: Vec<Packet>,
    responses_seen: usize,
    attempted: Vec<u8>,
    acked_zones: Vec<u8>,
    unmatched: BTreeMap<u8, usize>,
    aborted: bool,
    window: usize,
    clean_frames: u8,
    staged_zones: usize,
    busy_us_per_zone: f32,
    frames_since_probe: u8,
    /// Colour per LED on the device; `None` is `HIDPP20_UNCOMMITTED`.
    sent: Vec<Option<Color>>,
    resync_cursor: usize,
    retry_attempt: usize,
    retry_at: Option<Instant>,
    window_8080: usize,
    clean_frames_8080: u8,
}

fn rgb(c: Color) -> u32 {
    c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16
}

/// `LogitechHIDPP20Controller::NameLooksReal`.
pub fn name_looks_real(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.bytes().all(|c| (0x20..=0x7E).contains(&c))
}

impl<L: Link> Hidpp<L> {
    pub fn new(link: L, index: u8, wireless: bool, bluetooth: bool, pairing_name: String) -> Self {
        Hidpp {
            link,
            vl: None,
            index,
            wireless,
            bluetooth,
            pairing_name,
            caps: Caps::default(),
            wire_error: None,
            claimed: false,
            upgrade_to_5: false,
            prep_applied: false,
            pipelining: false,
            pipeline: Vec::new(),
            packets: Vec::new(),
            responses_seen: 0,
            attempted: Vec::new(),
            acked_zones: Vec::new(),
            unmatched: BTreeMap::new(),
            aborted: false,
            window: PERKEY_WINDOW_MAX,
            clean_frames: 0,
            staged_zones: 0,
            busy_us_per_zone: 0.0,
            frames_since_probe: 0,
            sent: Vec::new(),
            resync_cursor: 0,
            retry_attempt: 0,
            retry_at: None,
            window_8080: WINDOW_8080_MIN,
            clean_frames_8080: 0,
        }
    }

    /// `SendStandard`: Windows opens the long collection only, so every frame is a 0x11.
    fn send(&mut self, feat: u8, function: u8, data: &[u8]) -> Result<(), Fail> {
        let mut buf = [0u8; 20];
        buf[0] = LONG;
        buf[1] = self.index;
        buf[2] = feat;
        buf[3] = function | SW_ID;
        let n = data.len().min(16);
        buf[4..4 + n].copy_from_slice(&data[..n]);
        self.link.write(&buf).map(|_| ()).map_err(|e| {
            self.wire_error = Some(e);
            Fail::Wire
        })
    }

    /// `SendVeryLongFrame`: the answer lands on `link`.
    fn send_very_long(&mut self, feat: u8, function: u8, data: &[u8]) -> bool {
        let mut buf = [0u8; 64];
        buf[0] = VERY_LONG;
        buf[1] = self.index;
        buf[2] = feat;
        buf[3] = function | SW_ID;
        let n = data.len().min(60);
        buf[4..4 + n].copy_from_slice(&data[..n]);
        let link = match self.vl.as_mut() {
            Some(vl) => vl,
            None => &mut self.link,
        };
        link.write(&buf).is_ok()
    }

    /// `ReadStandardDirect`: the next HID++ frame for this device index, skipping everything else.
    fn read_message(&mut self, timeout_ms: u64) -> Result<Option<(u8, u8, [u8; 60])>, Fail> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut drained = 0;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let remaining = (deadline - now).as_millis() as i32;
            if remaining <= 0 {
                return Ok(None);
            }
            let mut buf = [0u8; 64];
            let n = self.link.read(&mut buf, remaining).map_err(|e| {
                self.wire_error = Some(e);
                Fail::Wire
            })?;
            if n == 0 {
                return Ok(None);
            }
            let hidpp = matches!(buf[0], 0x10..=0x12);
            if !hidpp || buf[1] != self.index {
                drained += 1;
                if drained > READ_DRAIN_BUDGET {
                    return Ok(None);
                }
                continue;
            }
            let mut data = [0u8; 60];
            data.copy_from_slice(&buf[4..64]);
            return Ok(Some((buf[2], buf[3], data)));
        }
    }

    /// Reads with a zero timeout until nothing is queued, at most `max` reports.
    fn drain(&mut self, max: usize, ms: i32) {
        let mut buf = [0u8; 64];
        for _ in 0..max {
            if self.link.read(&mut buf, ms).unwrap_or(0) == 0 {
                break;
            }
        }
    }

    /// `FlushResponseQueue` with no reader thread running.
    fn flush(&mut self) {
        self.drain(64, 0);
    }

    fn ours(feat: u8, function: u8, resp_feat: u8, resp_func: u8) -> bool {
        resp_feat == feat && resp_func & 0xF0 == function & 0xF0 && resp_func & 0x0F == SW_ID
    }

    /// `SendAcked`.
    fn acked(&mut self, feat: u8, function: u8, data: &[u8], policy: Policy) -> Reply {
        self.flush();
        let mut last = Fail::Timeout;
        for (attempt, &delay) in policy.backoff.iter().enumerate() {
            if delay > 0 {
                self.link.sleep(delay);
            }
            if self.send(feat, function, data).is_err() {
                last = Fail::Wire;
                continue;
            }
            let deadline = Instant::now() + Duration::from_millis(policy.window_ms);
            loop {
                let now = Instant::now();
                if now >= deadline {
                    last = Fail::Timeout;
                    break;
                }
                let Some((rf, rfn, rd)) = self.read_message((deadline - now).as_millis() as u64)?
                else {
                    last = Fail::Timeout;
                    break;
                };
                if rf == 0xFF {
                    if Self::ours(feat, function, rfn, rd[0]) {
                        if rd[1] == 0x08 {
                            last = Fail::Timeout;
                            break;
                        }
                        return Err(Fail::Error(rd[1]));
                    }
                    continue;
                }
                if Self::ours(feat, function, rf, rfn) {
                    if attempt > 0 {
                        self.drain_late(feat, function, attempt);
                    }
                    return Ok(rd);
                }
            }
        }
        Err(last)
    }

    /// `SendAckedIntoFAP`: during a pipelined claim the write goes out unanswered and is kept.
    fn fap(&mut self, feat: u8, function: u8, data: &[u8], policy: Policy) -> Reply {
        if self.pipelining {
            self.send(feat, function, data)?;
            if self.pipeline.len() < 16 {
                self.pipeline
                    .push((feat, function, data[..data.len().min(20)].to_vec()));
            }
            return Ok([0; 60]);
        }
        self.acked(feat, function, data, policy)
    }

    /// `DrainLateAnswers`.
    fn drain_late(&mut self, feat: u8, function: u8, expected: usize) {
        let deadline = Instant::now() + Duration::from_millis(LATE_ANSWER_GRACE_MS);
        let mut drained = 0;
        while drained < expected {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match self.read_message((deadline - now).as_millis() as u64) {
                Ok(Some((rf, rfn, _))) => {
                    if Self::ours(feat, function, rf, rfn) {
                        drained += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// `GetFeatureIndex`: IRoot lookup, cached when the device answered.
    pub fn feature(&mut self, page: u16, policy: Policy) -> u8 {
        if let Some(&idx) = self.caps.feature_map.get(&page) {
            return idx;
        }
        match self.acked(0x00, 0x00, &page.to_be_bytes(), policy) {
            Ok(r) => {
                self.caps.feature_map.insert(page, r[0]);
                r[0]
            }
            Err(_) => 0,
        }
    }

    fn first_contact(&self) -> Policy {
        if self.bluetooth {
            BLUETOOTH
        } else if self.wireless {
            FIRST_CONTACT
        } else {
            PROBE
        }
    }

    /// `ProbeIdentity`: the unit id, the same on every link the device has.
    pub fn probe_identity(&mut self) -> String {
        self.flush();
        if self.feature(FEAT_FEATURE_SET, self.first_contact()) == 0 {
            return String::new();
        }
        self.discover_firmware();
        if self.caps.unit_id.is_empty() && !self.caps.serial.is_empty() {
            return self.caps.serial.clone();
        }
        self.caps.unit_id.clone()
    }

    /// `Probe` on the standard transport.
    pub fn probe(&mut self) -> Probed {
        self.drain(20, 0);
        let mut test_idx = 0;
        for attempt in 0..3 {
            if test_idx != 0 {
                break;
            }
            if attempt > 0 {
                self.drain(usize::MAX, 10);
            }
            test_idx = self.feature(FEAT_FEATURE_SET, self.first_contact());
        }
        if test_idx == 0 {
            return Probed::NoAnswer;
        }
        self.drain(usize::MAX, 10);
        self.caps.feature_map.clear();
        self.caps.feature_map.insert(0x0000, 0);
        self.discover_name();
        self.discover_type();
        self.discover_firmware();
        self.caps.idx_profile_management = self.feature(FEAT_PROFILE_MANAGEMENT, RELIABLE);
        self.caps.idx_onboard_profiles = self.feature(FEAT_ONBOARD_PROFILES, RELIABLE);
        self.caps.idx_disable_keys = self.feature(FEAT_DISABLE_KEYS_BY_USAGE, RELIABLE);
        self.discover_rgb_effects();
        if self.caps.idx_rgb_effects == 0 && self.feature(FEAT_HEADSET_RGB_HOSTMODE, RELIABLE) != 0
        {
            return Probed::Unported(FEAT_HEADSET_RGB_HOSTMODE);
        }
        self.discover_perkey_zones();
        self.discover_perkey_8080();
        let layout = self.feature(FEAT_KEYBOARD_LAYOUT, RELIABLE);
        self.caps.layout_code = 0;
        if layout != 0 {
            self.caps.layout_code = self.acked(layout, 0x00, &[], RELIABLE).map_or(0, |r| r[0]);
        }
        self.feature(FEAT_WIRELESS_STATUS, RELIABLE);
        // LogFeatureSummary looks each of these up, uncached ones on the wire.
        for page in [
            FEAT_FEATURE_SET,
            FEAT_FIRMWARE_INFO,
            FEAT_DISABLE_KEYS_BY_USAGE,
            FEAT_COLOR_LED_EFFECTS,
            FEAT_RGB_EFFECTS,
            FEAT_PER_KEY_V1,
            FEAT_PER_KEY_V2,
            FEAT_KEYBOARD_LAYOUT,
            FEAT_ONBOARD_PROFILES,
            FEAT_PROFILE_MANAGEMENT,
            FEAT_CENTURION_RGB,
            FEAT_HEADSET_RGB_HOSTMODE,
            FEAT_WIRELESS_STATUS,
        ] {
            self.feature(page, RELIABLE);
        }
        if !self.caps.has_zone_effects && !self.caps.has_perkey && !self.caps.has_perkey_8080 {
            return Probed::NoLighting;
        }
        Probed::Lit
    }

    /// `DiscoverDeviceName`.
    fn discover_name(&mut self) {
        let idx = self.feature(FEAT_DEVICE_NAME_TYPE, RELIABLE);
        if idx != 0 {
            for _ in 0..3 {
                let Ok(r) = self.acked(idx, 0x01, &[], RELIABLE) else {
                    continue;
                };
                let len = r[0] as usize;
                if len == 0 || len > 64 {
                    continue;
                }
                let mut name = Vec::new();
                let mut complete = true;
                for offset in (0..len).step_by(16) {
                    match self.acked(idx, 0x11, &[offset as u8], RELIABLE) {
                        Ok(r) => name.extend_from_slice(&r[..(len - offset).min(16)]),
                        Err(_) => {
                            complete = false;
                            break;
                        }
                    }
                }
                while name.last() == Some(&0) {
                    name.pop();
                }
                let name = String::from_utf8_lossy(&name).into_owned();
                if complete && name_looks_real(&name) {
                    self.caps.name = name;
                    return;
                }
            }
        }
        self.caps.name = if name_looks_real(&self.pairing_name) {
            self.pairing_name.clone()
        } else {
            PLACEHOLDER.into()
        };
    }

    /// `DiscoverDeviceType`.
    fn discover_type(&mut self) {
        self.caps.device_type = DEVICE_TYPE_MOUSE;
        let idx = self.feature(FEAT_DEVICE_NAME_TYPE, RELIABLE);
        if idx == 0 {
            return;
        }
        for _ in 0..3 {
            if let Ok(r) = self.acked(idx, 0x21, &[], RELIABLE) {
                self.caps.device_type = r[0];
                return;
            }
        }
    }

    /// `DiscoverFirmwareInfo`: unit id, model ids for the quirk table, entity versions, serial.
    fn discover_firmware(&mut self) {
        let fw = self.feature(FEAT_FIRMWARE_INFO, RELIABLE);
        if fw == 0 {
            return;
        }
        let mut entities = 1u8;
        if let Ok(r) = self.acked(fw, 0x00, &[], RELIABLE) {
            entities = r[0];
            self.caps.unit_id = format!("{:02X}{:02X}{:02X}{:02X}", r[1], r[2], r[3], r[4]);
            let pids = [
                u16::from_be_bytes([r[7], r[8]]),
                u16::from_be_bytes([r[9], r[10]]),
                u16::from_be_bytes([r[11], r[12]]),
            ];
            if self.caps.serial.is_empty() && self.caps.unit_id != "00000000" {
                self.caps.serial = self.caps.unit_id.clone();
            }
            self.caps.pids = pids;
            // HIDPP20_QUIRK_KEEP_ONBOARD_MODE: G915 TKL, wireless 0x408E or wired 0xC343.
            self.caps.keep_onboard = pids.iter().any(|&p| p != 0 && (p == 0x408E || p == 0xC343));
        }
        for entity in 0..entities.min(8) {
            let _ = self.acked(fw, 0x10, &[entity], RELIABLE);
        }
        if let Ok(r) = self.acked(fw, 0x20, &[], RELIABLE) {
            let mut serial: Vec<u8> = r[..16].to_vec();
            while matches!(serial.last(), Some(0 | b' ')) {
                serial.pop();
            }
            if let Some(nul) = serial.iter().position(|&c| c == 0) {
                serial.truncate(nul);
            }
            if !serial.is_empty() {
                self.caps.serial = String::from_utf8_lossy(&serial).into_owned();
            }
        }
    }

    /// `DiscoverRGBEffects` and `DiscoverEffectCards`.
    fn discover_rgb_effects(&mut self) {
        let mut page = FEAT_RGB_EFFECTS;
        let mut idx = self.feature(page, RELIABLE);
        if idx == 0 {
            page = FEAT_CENTURION_RGB;
            idx = self.feature(page, RELIABLE);
        }
        if idx == 0 {
            page = FEAT_COLOR_LED_EFFECTS;
            idx = self.feature(page, RELIABLE);
        }
        self.caps.idx_rgb_effects = idx;
        self.caps.rgb_page = page;
        if idx == 0 {
            return;
        }
        let rgb2 = page == FEAT_RGB_EFFECTS;
        if page == FEAT_COLOR_LED_EFFECTS {
            (self.caps.fn_set_effect, self.caps.fn_sw_control) = (0x30, 0x80);
            (self.caps.fn_pwr_config, self.caps.fn_pwr_mode) = (0, 0);
            self.caps.has_power_mgmt = false;
            self.caps.sw_control_simple = true;
        } else {
            (self.caps.fn_set_effect, self.caps.fn_sw_control) = (0x10, 0x50);
            (self.caps.fn_pwr_config, self.caps.fn_pwr_mode) = (0x70, 0x80);
            self.caps.has_power_mgmt = true;
            self.caps.sw_control_simple = false;
        }
        let info: &[u8] = if rgb2 { &[0xFF, 0xFF, 0x00] } else { &[] };
        let Ok(r) = self.fap(idx, 0x00, info, RELIABLE) else {
            return;
        };
        let count = if rgb2 { r[2] } else { r[0] };
        for i in 0..count {
            let reply = if rgb2 {
                self.fap(idx, 0x00, &[i, 0xFF], RELIABLE)
            } else {
                self.fap(idx, 0x10, &[i, 0x00], RELIABLE)
            };
            let Ok(r) = reply else {
                continue;
            };
            let effect_count = if rgb2 { r[4] } else { r[3] };
            let mut cluster = Cluster {
                index: i,
                effects: Vec::new(),
            };
            for j in 0..effect_count {
                let function = if rgb2 { 0x00 } else { 0x20 };
                if let Ok(r) = self.fap(idx, function, &[i, j, 0, 0], RELIABLE) {
                    cluster.effects.push(Effect {
                        index: j,
                        id: u16::from_be_bytes([r[2], r[3]]),
                    });
                }
            }
            self.caps.clusters.push(cluster);
        }
        self.caps.has_zone_effects = !self.caps.clusters.is_empty();
        if rgb2 && let Ok(r) = self.fap(idx, 0x00, &[0xFF, 0x00, 0x01, 0x00, 0x01], PROBE) {
            self.caps.has_effect_cards = true;
            self.caps.template = [r[10], r[11]];
        }
    }

    /// `DiscoverPerKeyZones`: three 112-bit pages of 0x8081 zone ids.
    fn discover_perkey_zones(&mut self) {
        let idx = self.feature(FEAT_PER_KEY_V2, RELIABLE);
        self.caps.idx_perkey_v2 = idx;
        if idx == 0 {
            return;
        }
        for page in 0..3u8 {
            let Ok(r) = self.fap(idx, 0x00, &[0x00, page, 0x00], RELIABLE) else {
                continue;
            };
            let start = if page == 0 { 1 } else { 0 };
            for bit in start..112u16 {
                if r[2 + (bit / 8) as usize] & (1 << (bit % 8)) != 0 {
                    let zone = page as u16 * 112 + bit;
                    if zone <= 255 {
                        self.caps.perkey_zone_ids.push(zone);
                    }
                }
            }
        }
        self.caps.has_perkey = !self.caps.perkey_zone_ids.is_empty();
    }

    /// `DiscoverPerKey8080`: fn0 typeFlags, fn1 keyCount per set keyType, fn2 keyIds.
    fn discover_perkey_8080(&mut self) {
        self.caps.has_perkey_8080 = false;
        self.caps.has_numpad = false;
        self.caps.perkey_8080_types.clear();
        let idx = self.feature(FEAT_PER_KEY_V1, RELIABLE);
        self.caps.idx_perkey_8080 = idx;
        if idx == 0 {
            return;
        }
        let Ok(info) = self.acked(idx, FN_8080_GET_INFO, &[], RELIABLE) else {
            return;
        };
        let type_flags = u16::from_be_bytes([info[0], info[1]]);
        self.caps.perkey_8080_max_key_count = u16::from_be_bytes([info[5], info[6]]);
        for key_type in KEY_TYPES_8080 {
            if type_flags & key_type == 0 {
                continue;
            }
            let count = self
                .acked(
                    idx,
                    FN_8080_GET_KEY_TYPE_INFO,
                    &key_type.to_be_bytes(),
                    PROBE,
                )
                .map(|r| u16::from_be_bytes([r[0], r[1]]));
            if count == Ok(0) {
                continue;
            }
            let key_ids = self.read_key_ids_8080(idx, key_type, count.unwrap_or(0));
            if !key_ids.is_empty() {
                self.caps
                    .perkey_8080_types
                    .push(KeyType8080 { key_type, key_ids });
            }
        }
        self.caps.has_perkey_8080 = !self.caps.perkey_8080_types.is_empty();
        self.caps.has_numpad = self
            .caps
            .perkey_8080_types
            .iter()
            .any(|t| t.key_ids.iter().any(|id| (0x53..=0x63).contains(id)));
    }

    /// `ReadPerKey8080Colors`: GetKeyColors pages of 14; `expected` 0 pages until one is empty.
    fn read_key_ids_8080(&mut self, idx: u8, key_type: u16, expected: u16) -> Vec<u8> {
        let pages = if expected > 0 {
            (expected as usize).div_ceil(KEYS_PER_FRAME_8080)
        } else {
            (self.caps.perkey_8080_max_key_count as usize / KEYS_PER_FRAME_8080 + 4).max(16)
        };
        let mut ids = Vec::new();
        for page in 0..pages {
            let start = (page * KEYS_PER_FRAME_8080) as u16;
            let mut query = [0u8; 5];
            query[..2].copy_from_slice(&key_type.to_be_bytes());
            query[2..4].copy_from_slice(&start.to_be_bytes());
            let Ok(r) = self.acked(idx, FN_8080_GET_KEY_COLORS, &query, PROBE) else {
                break;
            };
            let found: Vec<u8> = r[4..]
                .chunks(4)
                .map(|e| e[0])
                .filter(|&id| id != 0)
                .collect();
            if found.is_empty() {
                break;
            }
            ids.extend(found);
        }
        ids
    }

    /// `SetSWControl`.
    fn set_sw_control(&mut self, mode: u8, flags: u8) -> Reply {
        if self.caps.idx_rgb_effects == 0 {
            return Err(Fail::Timeout);
        }
        let data = if self.caps.sw_control_simple {
            let en = (mode > 0) as u8;
            vec![en, en]
        } else {
            vec![0x01, mode, flags]
        };
        self.fap(
            self.caps.idx_rgb_effects,
            self.caps.fn_sw_control,
            &data,
            RELIABLE,
        )
    }

    /// `ClaimSWControlIfNeeded`, for the zone-effect pages this port drives.
    fn claim(&mut self) -> bool {
        if self.claimed {
            return true;
        }
        if self.caps.rgb_page == FEAT_COLOR_LED_EFFECTS && self.caps.idx_perkey_8080 != 0 {
            return true;
        }
        if self.caps.idx_rgb_effects == 0 {
            return false;
        }
        if self.caps.rgb_page == FEAT_COLOR_LED_EFFECTS {
            if self.set_sw_control(1, 1).is_err() {
                return false;
            }
            self.claimed = true;
            return true;
        }
        self.pipeline.clear();
        self.pipelining = true;
        if self.set_sw_control(3, 6).is_err() {
            self.pipelining = false;
            self.pipeline.clear();
            return false;
        }
        if self.caps.idx_disable_keys != 0 {
            let _ = self.fap(self.caps.idx_disable_keys, 0x30, &[], PROBE);
            let _ = self.fap(self.caps.idx_disable_keys, 0x10, &[], PROBE);
        }
        if self.caps.has_power_mgmt {
            let rgb = self.caps.idx_rgb_effects;
            let _ = self.fap(rgb, self.caps.fn_pwr_mode, &[0x01, 0x01], RELIABLE);
            let mut config = [0u8; 16];
            config[0] = 0x01;
            config[3..5].copy_from_slice(&IDLE_TIMEOUT_S.to_be_bytes());
            config[5..7].copy_from_slice(&SLEEP_TIMEOUT_S.to_be_bytes());
            let _ = self.fap(rgb, self.caps.fn_pwr_config, &config, RELIABLE);
        }
        if !self.caps.keep_onboard {
            self.set_host_mode();
        }
        self.pipelining = false;
        self.verify_pipeline();
        self.claimed = true;
        self.upgrade_to_5 = true;
        self.prep_applied = false;
        true
    }

    /// `SetHostMode`.
    fn set_host_mode(&mut self) {
        if self.caps.idx_profile_management != 0 {
            let _ = self.fap(self.caps.idx_profile_management, 0x60, &[0x05], RELIABLE);
        } else if self.caps.idx_onboard_profiles != 0 {
            let _ = self.fap(self.caps.idx_onboard_profiles, 0x10, &[0x02], RELIABLE);
        }
    }

    /// `VerifyClaimPipeline`: match the claim's ACKs in any order, re-send a missed one blocking.
    fn verify_pipeline(&mut self) {
        let pipeline = std::mem::take(&mut self.pipeline);
        let mut acked = vec![false; pipeline.len()];
        let mut got = 0;
        for _ in 0..pipeline.len() + 4 {
            if got >= pipeline.len() {
                break;
            }
            let Ok(Some((rf, rfn, _))) = self.read_message(60) else {
                break;
            };
            if rfn & 0x0F != SW_ID {
                continue;
            }
            if let Some(i) = (0..pipeline.len())
                .find(|&i| !acked[i] && rf == pipeline[i].0 && rfn & 0xF0 == pipeline[i].1 & 0xF0)
            {
                acked[i] = true;
                got += 1;
            }
        }
        for (i, (feat, function, data)) in pipeline.iter().enumerate() {
            if !acked[i] {
                let _ = self.fap(*feat, *function, data, RELIABLE);
            }
        }
    }

    /// `UpgradeSwControlAfterFirstPaint`: flags 6 to 5 once the per-key layer holds a picture.
    fn upgrade(&mut self) {
        if !self.upgrade_to_5 {
            return;
        }
        if self.caps.idx_rgb_effects == 0 {
            self.upgrade_to_5 = false;
            return;
        }
        if self.set_sw_control(3, 5).is_ok() {
            self.upgrade_to_5 = false;
        }
    }

    /// `SetZoneEffect` with the Static effect (0x0001), the only one direct mode uses.
    fn set_static(&mut self, cluster: u8, effect: u8, c: Color, persist: bool) {
        let mut data = [0u8; 16];
        data[0] = cluster;
        data[1] = effect;
        data[2..5].copy_from_slice(&[c.0, c.1, c.2]);
        if c != Color::BLACK {
            data[5] = 0x02;
        }
        data[12] = persist as u8;
        let _ = self.fap(
            self.caps.idx_rgb_effects,
            self.caps.fn_set_effect,
            &data,
            RELIABLE,
        );
    }

    /// `DoObservedPerKeyPrep`.
    fn observed_prep(&mut self) {
        let mut prep1 = [0u8; 16];
        prep1[..2].copy_from_slice(&[0xFF, 0x02]);
        prep1[8..10].copy_from_slice(&self.caps.template);
        prep1[12] = 0x01;
        let (rgb, set) = (self.caps.idx_rgb_effects, self.caps.fn_set_effect);
        let _ = self.fap(rgb, set, &prep1, RELIABLE);
        let effects = self
            .caps
            .clusters
            .first()
            .map_or(0, |c| c.effects.len().min(0xFF) as u8);
        let mut prep2 = [0u8; 16];
        prep2[..2].copy_from_slice(&[0xFF, effects]);
        prep2[12] = 0x01;
        let _ = self.fap(rgb, set, &prep2, RELIABLE);
    }

    /// `DoKeyboardFamilyPerKeyPrep`: every cluster Off, a black primer key, FrameEnd.
    fn keyboard_prep(&mut self) {
        let (rgb, set) = (self.caps.idx_rgb_effects, self.caps.fn_set_effect);
        let clusters: Vec<u8> = self.caps.clusters.iter().map(|c| c.index).collect();
        for cluster in clusters {
            let mut off = [0u8; 16];
            off[0] = cluster;
            off[12] = 0x01;
            let _ = self.fap(rgb, set, &off, RELIABLE);
        }
        let Some(&first) = self.caps.perkey_zone_ids.first() else {
            return;
        };
        let primer = first as u8;
        self.send_perkey_data(FN_SET_INDIVIDUAL, &[primer, 0, 0, 0], vec![primer]);
        self.frame_end();
    }

    /// `SendPerKeyData`: windowed, paced by the ACK stream.
    fn send_perkey_data(&mut self, function: u8, data: &[u8], zones: Vec<u8>) {
        self.attempted.extend(&zones);
        if self.aborted {
            return;
        }
        while self.packets.len().saturating_sub(self.responses_seen) >= self.window {
            if !self.process_one(PERKEY_ACK_WAIT_MS) {
                self.aborted = true;
                return;
            }
        }
        if self.send(self.caps.idx_perkey_v2, function, data).is_err() {
            return;
        }
        let mut payload = [0u8; 16];
        let len = data.len().min(16);
        payload[..len].copy_from_slice(&data[..len]);
        self.staged_zones += zones.len();
        self.packets.push(Packet {
            function,
            payload,
            len,
            zones,
            acked: false,
        });
    }

    /// `ProcessOnePerKeyResponse`; false when nothing arrived.
    fn process_one(&mut self, timeout_ms: u64) -> bool {
        let Ok(Some((rf, rfn, rd))) = self.read_message(timeout_ms) else {
            return false;
        };
        let idx = self.caps.idx_perkey_v2;
        if rf == 0xFF {
            if rfn == idx && rd[0] & 0x0F == SW_ID && rd[0] & 0xF0 != FN_FRAME_END {
                self.responses_seen += 1;
            }
            return true;
        }
        if rf != idx || rfn & 0x0F != SW_ID || rfn & 0xF0 == FN_FRAME_END {
            return true;
        }
        self.responses_seen += 1;
        self.attribute_ack(rfn & 0xF0, &rd);
        true
    }

    fn attribute_ack(&mut self, function: u8, rd: &[u8; 60]) {
        if !self.match_ack(function, rd) {
            *self.unmatched.entry(function).or_default() += 1;
        }
    }

    /// `MatchPerKeyAck`: which packet an ACK echoes.
    fn match_ack(&mut self, function: u8, rd: &[u8; 60]) -> bool {
        for p in self.packets.iter_mut() {
            if p.acked || p.function != function {
                continue;
            }
            let matched = match p.function {
                FN_SET_INDIVIDUAL => {
                    let entries = p.len / 4;
                    let echo: Vec<(u8, u8)> = rd
                        .iter()
                        .copied()
                        .zip(p.payload.iter().copied().step_by(4))
                        .take(entries)
                        .collect();
                    let mut ok = true;
                    let mut echoed = 0;
                    for &(r, z) in &echo {
                        if r == z {
                            echoed += 1;
                        } else if r != 0xFF {
                            ok = false;
                            break;
                        }
                    }
                    if !ok || echoed == 0 {
                        continue;
                    }
                    for &(r, z) in &echo {
                        if r == z {
                            self.acked_zones.push(z);
                        }
                    }
                    p.acked = true;
                    return true;
                }
                FN_SET_RANGE => {
                    let entries = p.len / 5;
                    entries > 0 && (0..entries).all(|k| rd[k] == p.payload[k * 5])
                }
                FN_SET_SINGLE_VALUE => rd[..3] == p.payload[..3] && p.zones.contains(&rd[3]),
                _ => rd[..p.len] == p.payload[..p.len],
            };
            if matched {
                self.acked_zones.extend(&p.zones);
                p.acked = true;
                return true;
            }
        }
        false
    }

    /// `PerKeyFrameEnd`: collect the write ACKs, then FrameEnd, polling through BUSY.
    fn frame_end(&mut self) -> FrameResult {
        let mut result = FrameResult {
            attempted: std::mem::take(&mut self.attempted),
            ..FrameResult::default()
        };
        let idx = self.caps.idx_perkey_v2;
        if idx == 0 {
            result.acked = std::mem::take(&mut self.acked_zones);
            self.reset_frame();
            return result;
        }
        if !self.aborted {
            let deadline = Instant::now() + Duration::from_millis(250);
            while self.responses_seen < self.packets.len() {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let remaining = ((deadline - now).as_millis() as u64).min(PERKEY_ACK_TAIL_MS);
                if remaining == 0 || !self.process_one(remaining) {
                    break;
                }
            }
        }
        let acks_done = Instant::now();
        let mut busy_polls = 0;
        let mut predicted_ms = 0u16;
        let mut fe_probe = false;
        let commit_zones = self.staged_zones;
        if !self.aborted {
            let predicted = (self.busy_us_per_zone * commit_zones as f32 / 1000.0).min(100.0);
            predicted_ms = predicted as u16;
            if predicted_ms > FE_PROBE_STEP_MS && {
                self.frames_since_probe += 1;
                self.frames_since_probe >= FE_PROBE_INTERVAL
            } {
                self.frames_since_probe = 0;
                predicted_ms -= FE_PROBE_STEP_MS;
                fe_probe = true;
            }
            if predicted_ms > 0 {
                self.link.sleep(predicted_ms as u64);
            }
            let zeros = [0u8; 16];
            if self.send(idx, FN_FRAME_END, &zeros).is_ok() {
                let deadline = Instant::now() + Duration::from_millis(250);
                while !result.frame_end_acked {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    let Ok(Some((rf, rfn, rd))) =
                        self.read_message((deadline - now).as_millis() as u64)
                    else {
                        break;
                    };
                    if rf == 0xFF {
                        if rfn != idx || rd[0] & 0x0F != SW_ID {
                            continue;
                        }
                        if rd[0] & 0xF0 != FN_FRAME_END {
                            self.responses_seen += 1;
                            continue;
                        }
                        if rd[1] == 0x08 {
                            busy_polls += 1;
                            self.link.sleep(if predicted_ms > 0 {
                                FRAME_END_BUSY_POLL_NEAR_MS
                            } else {
                                FRAME_END_BUSY_POLL_MS
                            });
                            let _ = self.send(idx, FN_FRAME_END, &zeros);
                            continue;
                        }
                        break;
                    }
                    if rf != idx || rfn & 0x0F != SW_ID {
                        continue;
                    }
                    if rfn & 0xF0 == FN_FRAME_END {
                        result.frame_end_acked = true;
                        break;
                    }
                    self.responses_seen += 1;
                    self.attribute_ack(rfn & 0xF0, &rd);
                }
            }
        }
        if result.frame_end_acked {
            self.staged_zones = 0;
        }
        if result.frame_end_acked && commit_zones > 0 {
            let zones = commit_zones as f32;
            if fe_probe {
                if busy_polls == 0 {
                    self.busy_us_per_zone =
                        0.7 * self.busy_us_per_zone + 0.3 * (predicted_ms as f32 * 1000.0 / zones);
                }
            } else if busy_polls > 0 {
                let ready_ms = acks_done.elapsed().as_millis() as f32;
                self.busy_us_per_zone =
                    (0.7 * self.busy_us_per_zone + 0.3 * (ready_ms * 1000.0 / zones)).min(2000.0);
            }
        }
        for (&function, &count) in &self.unmatched {
            let unacked = self
                .packets
                .iter()
                .filter(|p| !p.acked && p.function == function)
                .count();
            if count == unacked {
                for p in self.packets.iter_mut() {
                    if !p.acked && p.function == function {
                        p.acked = true;
                        self.acked_zones.extend(&p.zones);
                    }
                }
            }
        }
        result.acked = std::mem::take(&mut self.acked_zones);
        if !self.packets.is_empty() {
            if self.packets.iter().any(|p| !p.acked) {
                self.clean_frames = 0;
                self.window = (self.window / 2).max(PERKEY_WINDOW_MIN);
            } else if self.window < PERKEY_WINDOW_MAX {
                self.clean_frames += 1;
                if self.clean_frames >= PERKEY_WINDOW_GROW_AFTER {
                    self.clean_frames = 0;
                    self.window += 1;
                }
            }
        }
        let complete = result.frame_end_acked && result.acked.len() == result.attempted.len();
        if !complete {
            self.drain_stale();
        }
        self.reset_frame();
        result
    }

    /// `DrainStaleResponses`.
    fn drain_stale(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            if !matches!(self.read_message(25), Ok(Some(_))) {
                return;
            }
        }
    }

    fn reset_frame(&mut self) {
        self.attempted.clear();
        self.packets.clear();
        self.acked_zones.clear();
        self.unmatched.clear();
        self.responses_seen = 0;
        self.aborted = false;
    }

    /// `SetPerKeyColors`: SetIndividualRgbZones, four zones a packet.
    fn set_perkey_colors(&mut self, pairs: &[(u8, Color)]) {
        for chunk in pairs.chunks(4) {
            let mut data = Vec::with_capacity(16);
            for &(zone, c) in chunk {
                data.extend([zone, c.0, c.1, c.2]);
            }
            let zones = chunk.iter().map(|&(z, _)| z).collect();
            self.send_perkey_data(FN_SET_INDIVIDUAL, &data, zones);
        }
    }

    /// `SendPerKeyFrame` for 0x8081; `zones[i]` is the zone id of LED `i`.
    fn perkey_frame(&mut self, zones: &[u16], snapshot: &[Color]) {
        self.claim();
        if self.upgrade_to_5 && !self.prep_applied && self.caps.has_zone_effects {
            self.prep_applied = true;
            let page_8071 = self.caps.rgb_page == FEAT_RGB_EFFECTS;
            if self.caps.idx_disable_keys != 0 && self.caps.idx_perkey_v2 != 0 && page_8071 {
                self.keyboard_prep();
            } else if self.caps.has_effect_cards && page_8071 {
                self.observed_prep();
            } else {
                let primers: Vec<(u8, u8)> = self
                    .caps
                    .clusters
                    .iter()
                    .filter_map(|c| {
                        c.effects
                            .iter()
                            .find(|e| e.id == 0x0001)
                            .map(|e| (c.index, e.index))
                    })
                    .collect();
                for (cluster, effect) in primers {
                    self.set_static(cluster, effect, Color::BLACK, true);
                }
            }
            self.sent.clear();
        }
        let full = self.sent.len() != snapshot.len();
        let mut by_color: BTreeMap<u32, (Color, Vec<u8>)> = BTreeMap::new();
        let mut in_frame = [false; 256];
        let mut delta = 0;
        let mapped = |z: u16| z != 0 && z <= 255;
        for (i, (&zone, &c)) in zones.iter().zip(snapshot).enumerate() {
            if !mapped(zone) {
                continue;
            }
            if full || self.sent[i] != Some(c) {
                by_color
                    .entry(rgb(c))
                    .or_insert((c, Vec::new()))
                    .1
                    .push(zone as u8);
                in_frame[zone as usize] = true;
                delta += 1;
            }
        }
        if !full && !zones.is_empty() {
            let budget = if delta <= RESYNC_QUIET_ZONES {
                RESYNC_KEYS_MAX
            } else {
                RESYNC_KEYS_MIN
            };
            let mut added = 0;
            for _ in 0..zones.len() {
                if added >= budget {
                    break;
                }
                let i = self.resync_cursor % zones.len();
                self.resync_cursor += 1;
                if i >= snapshot.len() || !mapped(zones[i]) || in_frame[zones[i] as usize] {
                    continue;
                }
                let c = snapshot[i];
                by_color
                    .entry(rgb(c))
                    .or_insert((c, Vec::new()))
                    .1
                    .push(zones[i] as u8);
                in_frame[zones[i] as usize] = true;
                added += 1;
            }
        }
        if by_color.is_empty() {
            return;
        }
        self.flush();
        let mut individual = Vec::new();
        for (_, (c, mut ids)) in by_color {
            if ids.len() < 2 {
                individual.extend(ids.into_iter().map(|z| (z, c)));
                continue;
            }
            ids.sort_unstable();
            let mut ranges = Vec::new();
            let mut scattered = Vec::new();
            let mut run_start = 0;
            for i in 1..=ids.len() {
                if i < ids.len() && ids[i] as u16 == ids[i - 1] as u16 + 1 {
                    continue;
                }
                if i - run_start >= 3 {
                    ranges.push((ids[run_start], ids[i - 1]));
                } else {
                    scattered.extend(&ids[run_start..i]);
                }
                run_start = i;
            }
            for batch in ranges.chunks(3) {
                let mut data = Vec::new();
                let mut packet_zones = Vec::new();
                for &(a, b) in batch {
                    data.extend([a, b, c.0, c.1, c.2]);
                    packet_zones.extend(a..=b);
                }
                self.send_perkey_data(FN_SET_RANGE, &data, packet_zones);
            }
            for batch in scattered.chunks(13) {
                let mut data = vec![c.0, c.1, c.2];
                data.extend(batch);
                self.send_perkey_data(FN_SET_SINGLE_VALUE, &data, batch.to_vec());
            }
        }
        if !individual.is_empty() {
            self.set_perkey_colors(&individual);
        }
        let commit = self.frame_end();
        if self.sent.len() != snapshot.len() {
            self.sent = vec![None; snapshot.len()];
        }
        let led_of: HashMap<u8, usize> = zones
            .iter()
            .enumerate()
            .filter(|(_, z)| mapped(**z))
            .map(|(i, &z)| (z as u8, i))
            .collect();
        for zone in &commit.attempted {
            if let Some(&i) = led_of.get(zone)
                && i < self.sent.len()
            {
                let ok = commit.frame_end_acked && commit.acked.contains(zone);
                self.sent[i] = ok.then_some(snapshot[i]);
            }
        }
        let complete = commit.frame_end_acked
            && zones
                .iter()
                .zip(&self.sent)
                .all(|(&z, s)| !mapped(z) || s.is_some());
        if complete {
            self.upgrade();
            self.cancel_retry();
        } else if self.upgrade_to_5 {
            if !self.schedule_retry() {
                self.upgrade();
                self.cancel_retry();
            }
        } else {
            self.cancel_retry();
        }
    }

    /// `ScheduleRetryPaint`; false once the backoff is spent.
    fn schedule_retry(&mut self) -> bool {
        match REPAINT_RETRY_BACKOFF_MS.get(self.retry_attempt) {
            Some(&wait) => {
                self.retry_at = Some(Instant::now() + Duration::from_millis(wait));
                self.retry_attempt += 1;
                true
            }
            None => {
                self.retry_at = None;
                false
            }
        }
    }

    fn cancel_retry(&mut self) {
        self.retry_at = None;
        self.retry_attempt = 0;
    }

    /// The 0x8080 branch of `DeviceUpdateLEDs`; `keys[i]` is LED `i`'s `keyType << 8 | keyId`.
    fn perkey_8080_frame(&mut self, keys: &[u16], snapshot: &[Color]) {
        self.claim();
        let full = self.sent.len() != snapshot.len();
        if full {
            self.sent = vec![None; snapshot.len()];
        }
        let n = snapshot.len().min(keys.len());
        let mut frame = Vec::new();
        let mut in_frame = vec![false; n];
        for i in 0..n {
            if keys[i] != 0 && self.sent[i] != Some(snapshot[i]) {
                frame.push(i);
                in_frame[i] = true;
            }
        }
        if !full && n > 0 {
            let budget = if frame.len() <= RESYNC_QUIET_ZONES {
                RESYNC_KEYS_MAX
            } else {
                RESYNC_KEYS_MIN
            };
            let mut added = 0;
            for _ in 0..n {
                if added >= budget {
                    break;
                }
                let i = self.resync_cursor % n;
                self.resync_cursor += 1;
                if in_frame[i] || keys[i] == 0 {
                    continue;
                }
                frame.push(i);
                in_frame[i] = true;
                added += 1;
            }
        }
        let mut by_type: Vec<(u16, Vec<(u8, Color)>)> = Vec::new();
        for &i in &frame {
            let (key_type, key) = (keys[i] >> 8, (keys[i] & 0xFF) as u8);
            match by_type.iter_mut().find(|(t, _)| *t == key_type) {
                Some((_, group)) => group.push((key, snapshot[i])),
                None => by_type.push((key_type, vec![(key, snapshot[i])])),
            }
        }
        if by_type.is_empty() {
            self.cancel_retry();
            return;
        }
        let landed = self.set_perkey_8080(&by_type);
        for &i in &frame {
            self.sent[i] = landed.then_some(snapshot[i]);
        }
        if landed {
            self.cancel_retry();
        } else {
            self.schedule_retry();
        }
    }

    /// `SetPerKey8080`: true only when every SetKeyColors write and the FlushLEDs were acked.
    fn set_perkey_8080(&mut self, by_type: &[(u16, Vec<(u8, Color)>)]) -> bool {
        let idx = self.caps.idx_perkey_8080;
        if idx == 0 {
            return false;
        }
        self.flush();
        let kt40_id12_only = self.caps.pids.contains(&KEYTYPE40_ID12_ONLY);
        let (mut sent, mut answered, mut rejected) = (0, 0, 0);
        let mut send_failed = false;
        let mut aborted = false;
        'types: for (key_type, keys) in by_type {
            let keys: Vec<(u8, Color)> = keys
                .iter()
                .copied()
                .filter(|&(id, _)| !(kt40_id12_only && *key_type == 0x40) || id == 1 || id == 2)
                .collect();
            for chunk in keys.chunks(KEYS_PER_FRAME_8080) {
                let mut data = Vec::with_capacity(60);
                data.extend(key_type.to_be_bytes());
                data.extend((chunk.len() as u16).to_be_bytes());
                for &(id, c) in chunk {
                    data.extend([id, c.0, c.1, c.2]);
                }
                while answered < sent && sent - answered >= self.window_8080 {
                    if !self.process_one_8080(PERKEY_ACK_WAIT_MS, &mut answered, &mut rejected) {
                        aborted = true;
                        break 'types;
                    }
                }
                if !self.send_very_long(idx, FN_8080_SET_KEY_COLORS, &data) {
                    send_failed = true;
                    continue;
                }
                sent += 1;
            }
        }
        if !aborted {
            let deadline = Instant::now() + Duration::from_millis(250);
            while answered < sent {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let remaining = ((deadline - now).as_millis() as u64).min(ACK_TAIL_8080_MS);
                if remaining == 0 || !self.process_one_8080(remaining, &mut answered, &mut rejected)
                {
                    break;
                }
            }
        }
        let committed = !aborted && sent > 0 && self.commit_8080(&mut answered, &mut rejected);
        let lost = sent.saturating_sub(answered);
        if lost > 0 {
            self.clean_frames_8080 = 0;
            self.window_8080 = (self.window_8080 / 2).max(WINDOW_8080_MIN);
        } else if sent > 0 && self.window_8080 < WINDOW_8080_MAX {
            self.clean_frames_8080 += 1;
            if self.clean_frames_8080 >= PERKEY_WINDOW_GROW_AFTER {
                self.clean_frames_8080 = 0;
                self.window_8080 += 1;
            }
        }
        committed && !send_failed && lost == 0 && rejected == 0
    }

    /// `ProcessOne8080Response`: counts a SetKeyColors ACK or error; false when nothing came.
    fn process_one_8080(
        &mut self,
        timeout_ms: u64,
        answered: &mut usize,
        rejected: &mut usize,
    ) -> bool {
        let Ok(Some((rf, rfn, rd))) = self.read_message(timeout_ms) else {
            return false;
        };
        let idx = self.caps.idx_perkey_8080;
        if rf == 0xFF {
            if rfn == idx && rd[0] == FN_8080_SET_KEY_COLORS | SW_ID {
                *answered += 1;
                *rejected += 1;
            }
        } else if rf == idx && rfn == FN_8080_SET_KEY_COLORS | SW_ID {
            *answered += 1;
        }
        true
    }

    /// `PerKeyCommit8080`: FlushLEDs on `link`, polling through BUSY.
    fn commit_8080(&mut self, answered: &mut usize, rejected: &mut usize) -> bool {
        let idx = self.caps.idx_perkey_8080;
        if self.send(idx, FN_8080_FLUSH_LEDS, &[]).is_err() {
            return false;
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let Ok(Some((rf, rfn, rd))) = self.read_message((deadline - now).as_millis() as u64)
            else {
                return false;
            };
            if rf == 0xFF && rfn == idx && rd[0] & 0x0F == SW_ID {
                match rd[0] & 0xF0 {
                    FN_8080_SET_KEY_COLORS => {
                        *answered += 1;
                        *rejected += 1;
                    }
                    FN_8080_FLUSH_LEDS if rd[1] == 0x08 => {
                        self.link.sleep(FRAME_END_BUSY_POLL_MS);
                        let _ = self.send(idx, FN_8080_FLUSH_LEDS, &[]);
                    }
                    FN_8080_FLUSH_LEDS => return false,
                    _ => {}
                }
                continue;
            }
            if rf != idx || rfn & 0x0F != SW_ID {
                continue;
            }
            match rfn & 0xF0 {
                FN_8080_FLUSH_LEDS => return true,
                FN_8080_SET_KEY_COLORS => *answered += 1,
                _ => {}
            }
        }
    }

    /// The zone-effect branch of `DeviceUpdateLEDs`: Direct rides Static, one LED per cluster.
    fn zone_frame(&mut self, colors: &[Color]) {
        self.claim();
        let full = self.sent.len() != colors.len();
        for (i, &c) in colors.iter().enumerate().take(self.caps.clusters.len()) {
            if !full && self.sent[i] == Some(c) {
                continue;
            }
            let cluster = &self.caps.clusters[i];
            let Some(effect) = cluster.effects.iter().find(|e| e.id == 0x0001) else {
                continue;
            };
            let (index, effect) = (cluster.index, effect.index);
            self.set_static(index, effect, c, false);
        }
        self.sent = colors.iter().map(|&c| Some(c)).collect();
        self.upgrade();
    }

    /// `DeviceUpdateLEDs` in Direct mode. Unchanged colours are skipped unless a first paint
    /// is still waiting for its retry, which OpenRGB's power thread would run.
    pub fn update(&mut self, zones: &[u16], colors: &[Color], changed: bool) -> Result<(), String> {
        let retry = self.retry_at.is_some_and(|t| Instant::now() >= t);
        if !changed && !retry {
            return Ok(());
        }
        if perkey_8080(&self.caps) {
            self.perkey_8080_frame(zones, colors);
        } else if self.caps.has_perkey {
            self.perkey_frame(zones, colors);
        } else {
            self.zone_frame(colors);
        }
        match self.wire_error.take() {
            Some(e) => {
                self.claimed = false;
                self.sent.clear();
                Err(e)
            }
            None => Ok(()),
        }
    }
}

fn kind(device_type: u8) -> DeviceKind {
    match device_type {
        0 | 2 => DeviceKind::Keyboard,
        3 | 5 => DeviceKind::Mouse,
        _ => DeviceKind::Other,
    }
}

/// The KeyboardLayoutManager full board as HID usages (`hidpp20_key_name_to_usage`), 0 empty.
#[rustfmt::skip]
const BOARD_8080: [[u8; 21]; 6] = [
    [0x29, 0, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0, 0, 0, 0],
    [0x35, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A, 0x49, 0x4A, 0x4B, 0x53, 0x54, 0x55, 0x56],
    [0x2B, 0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x31, 0x4C, 0x4D, 0x4E, 0x5F, 0x60, 0x61, 0x57],
    [0x39, 0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x33, 0x34, 0x32, 0x28, 0, 0, 0, 0x5C, 0x5D, 0x5E, 0],
    [0xE1, 0x64, 0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0, 0xE5, 0, 0x52, 0, 0x59, 0x5A, 0x5B, 0x58],
    [0xE0, 0xE3, 0xE2, 0, 0, 0, 0x2C, 0, 0, 0, 0xE6, 0xE7, 0x65, 0xE4, 0x50, 0x51, 0x4F, 0, 0x62, 0x63, 0],
];

/// Strip key: row, column, `keyType << 8 | keyId`, and whether it opens a new row.
type StripKey = (u8, u8, u16, bool);

const G810_STRIP: &[StripKey] = &[
    (0, 0, 0x1001, true),
    (0, 12, 0x4005, false),
    (0, 13, 0x4003, false),
    (0, 14, 0x4004, false),
    (0, 16, 0x4002, false),
    (0, 18, 0x4001, false),
    (0, 19, 0x02E2, false),
    (1, 17, 0x02CD, false),
    (1, 18, 0x02B7, false),
    (1, 19, 0x02B6, false),
    (1, 20, 0x02B5, false),
];

const GPRO_STRIP: &[StripKey] = &[
    (0, 0, 0x1001, true),
    (0, 12, 0x4003, false),
    (0, 13, 0x4004, false),
    (0, 15, 0x4002, false),
    (0, 16, 0x4001, false),
];

const G512_STRIP: &[StripKey] = &[(0, 17, 0x4003, false), (0, 18, 0x4002, false)];

const G910_STRIP: &[StripKey] = &[
    (0, 3, 0x0406, true),
    (0, 4, 0x0407, false),
    (0, 5, 0x0408, false),
    (0, 6, 0x0409, false),
    (1, 0, 0x1001, false),
    (2, 0, 0x0401, false),
    (3, 0, 0x0402, false),
    (4, 0, 0x0403, false),
    (5, 0, 0x0404, false),
    (6, 0, 0x0405, false),
    (7, 3, 0x1002, false),
];

/// `known_kb_8080_strips`: model id, full size, strip. Unclaimed enumerated ids are phantoms.
const STRIPS_8080: [(u16, bool, &[StripKey]); 9] = [
    (0xC331, true, G810_STRIP),
    (0xC337, true, G810_STRIP),
    (0xC333, true, G810_STRIP),
    (0xC338, true, G810_STRIP),
    (0xC342, true, G512_STRIP),
    (0xC33C, true, G512_STRIP),
    (0xC32B, true, G910_STRIP),
    (0xC335, true, G910_STRIP),
    (0xC339, false, GPRO_STRIP),
];

/// `FindKb8080Strip` on the wireless and wired model ids.
fn strip_8080(caps: &Caps) -> Option<(bool, &'static [StripKey])> {
    STRIPS_8080
        .iter()
        .find(|s| s.0 == caps.pids[0] || s.0 == caps.pids[1])
        .map(|s| (s.1, s.2))
}

/// `PerKey8080Capable`: enumerated keys, or a known keyboard whose strip addresses itself.
fn perkey_8080(caps: &Caps) -> bool {
    caps.has_perkey_8080
        || (caps.device_type == 0 && caps.idx_perkey_8080 != 0 && strip_8080(caps).is_some())
}

/// Whether `hidpp20_key_name_for_usage` names this keyId.
fn named_usage(id: u8) -> bool {
    matches!(id, 0x04..=0x65 | 0x85 | 0x87 | 0xE0..=0xE7)
}

/// The KLM board for `hidpp20_derive_keyboard_layout`, with the ABNT2 wire overrides.
fn board_8080(layout_code: u8, full: bool) -> Vec<(u8, u8, u16)> {
    let dropped: &[u8] = match layout_code {
        0x02 | 0x03 | 0x08 | 0x0E | 0x0F | 0x16 | 0x1A | 0x1D | 0x1F | 0x21 | 0x24 | 0x37 => {
            &[0x31]
        }
        0x38 => &[0x31],
        0x04 | 0x0D | 0x14 | 0x19 | 0x41 | 0x05 | 0x11 | 0x0A => &[],
        _ => &[0x32, 0x64],
    };
    let mut keys = Vec::new();
    for (r, row) in BOARD_8080.iter().enumerate() {
        for (c, &usage) in row.iter().enumerate() {
            if usage != 0 && (full || c < 17) && !dropped.contains(&usage) {
                keys.push((r as u8, c as u8, usage as u16));
            }
        }
    }
    if layout_code == 0x38 {
        for k in keys.iter_mut().filter(|k| (k.0, k.1) == (4, 11)) {
            k.2 = 0x0138;
        }
        keys.push((4, 12, 0x0188));
    }
    keys
}

/// `SetupZones8080`: matrix then Extras on a row below; values `keyType << 8 | keyId`, 0 for none.
fn layout_8080(caps: &Caps) -> (Vec<u16>, Vec<[f32; 2]>, [u32; 2]) {
    let strip = strip_8080(caps);
    let mut kb: Option<&KeyType8080> = None;
    if caps.device_type == 0 {
        let mut best = 0;
        for t in &caps.perkey_8080_types {
            let overlap = t.key_ids.iter().filter(|&&id| named_usage(id)).count();
            if overlap > best {
                best = overlap;
                kb = Some(t);
            }
        }
    }
    let mut matrix: Vec<(u8, u8, u16)> = Vec::new();
    if kb.is_some() || strip.is_some() {
        let full = match (kb, strip) {
            (None, Some((full, _))) => full,
            _ => caps.has_numpad,
        };
        matrix = board_8080(caps.layout_code, full);
        for &(row, col, value, new_row) in strip.map_or(&[][..], |s| s.1) {
            for k in matrix.iter_mut() {
                if new_row && k.0 >= row {
                    k.0 += 1;
                } else if !new_row && k.0 == row && k.1 >= col {
                    k.1 += 1;
                }
            }
            matrix.push((row, col, value));
        }
        matrix.sort_by_key(|k| (k.0, k.1));
        let key_type = kb.map_or(0x0001, |t| t.key_type);
        for k in matrix.iter_mut().filter(|k| k.2 < 0x100) {
            let usage = k.2 as u8;
            k.2 = match kb {
                Some(t) if !t.key_ids.contains(&usage) => 0,
                _ => key_type << 8 | usage as u16,
            };
        }
    }
    let claimed: Vec<u16> = matrix.iter().map(|k| k.2).filter(|&v| v != 0).collect();
    let mut extras: Vec<u16> = Vec::new();
    if kb.is_none() && strip.is_some() {
        for usage in [0x31, 0x32, 0x64] {
            if !claimed.contains(&(0x0100 | usage)) {
                extras.push(0x0100 | usage);
            }
        }
    }
    if strip.is_none() {
        for t in &caps.perkey_8080_types {
            for &id in &t.key_ids {
                let value = t.key_type << 8 | id as u16;
                if !claimed.contains(&value) {
                    extras.push(value);
                }
            }
        }
    }
    let line = |n: usize, y: f32| (0..n).map(move |i| [(i as f32 + 0.5) / n as f32, y]);
    if matrix.is_empty() {
        let n = extras.len();
        return (extras, line(n, 0.5).collect(), [n as u32, 1]);
    }
    let rows = matrix.iter().map(|k| k.0).max().unwrap_or(0) as u32 + 1;
    let cols = matrix.iter().map(|k| k.1).max().unwrap_or(0) as u32 + 1;
    let total = rows + !extras.is_empty() as u32;
    let mut leds: Vec<[f32; 2]> = matrix
        .iter()
        .map(|k| {
            [
                (k.1 as f32 + 0.5) / cols as f32,
                (k.0 as f32 + 0.5) / total as f32,
            ]
        })
        .collect();
    leds.extend(line(extras.len(), (rows as f32 + 0.5) / total as f32));
    let mut values: Vec<u16> = matrix.iter().map(|k| k.2).collect();
    values.extend(extras);
    (values, leds, [cols, total])
}

/// LED order and positions: per-key zones in discovery order, else one LED per cluster.
/// Keyboard zones sit where hidpp.rs's TKL table puts them; the rest go on a row below.
fn layout(caps: &Caps) -> (Vec<u16>, Vec<[f32; 2]>, [u32; 2]) {
    if perkey_8080(caps) {
        return layout_8080(caps);
    }
    if !caps.has_perkey {
        let n = caps.clusters.len().max(1);
        let leds = (0..caps.clusters.len())
            .map(|i| [(i as f32 + 0.5) / n as f32, 0.5])
            .collect();
        return (Vec::new(), leds, [n as u32, 1]);
    }
    let zones = caps.perkey_zone_ids.clone();
    if kind(caps.device_type) == DeviceKind::Keyboard {
        let tkl = crate::hidpp::tkl_layout();
        let extra: Vec<u16> = zones
            .iter()
            .copied()
            .filter(|&z| !tkl.iter().any(|(t, _)| *t as u16 == z))
            .collect();
        let rows = if extra.is_empty() { 6.0 } else { 7.0 };
        let leds = zones
            .iter()
            .map(|&z| match tkl.iter().find(|(t, _)| *t as u16 == z) {
                Some((_, [x, y])) => [*x, y * 6.0 / rows],
                None => {
                    let i = extra.iter().position(|&e| e == z).unwrap_or(0);
                    [(i as f32 + 0.5) / extra.len() as f32, 6.5 / rows]
                }
            })
            .collect();
        return (zones, leds, [18, rows as u32]);
    }
    let n = zones.len();
    let leds = (0..n).map(|i| [(i as f32 + 0.5) / n as f32, 0.5]).collect();
    (zones, leds, [n as u32, 1])
}

pub struct Unified<L: Link> {
    pub ctl: Hidpp<L>,
    device: Device,
    zones: Vec<u16>,
    last: Option<Vec<Color>>,
}

impl<L: Link> Unified<L> {
    pub fn new(ctl: Hidpp<L>, identity: &str) -> Self {
        let (zones, leds, grid) = layout(&ctl.caps);
        let kind = kind(ctl.caps.device_type);
        let device = Device {
            id: format!("hid:hidpp20-{identity}"),
            name: ctl.caps.name.clone(),
            kind,
            rect: rect(kind),
            grid,
            leds,
        };
        Unified {
            ctl,
            device,
            zones,
            last: None,
        }
    }
}

impl<L: Link + 'static> Backend for Unified<L> {
    fn name(&self) -> &str {
        "Logitech HID++ 2.0"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let colors: Vec<Color> = (0..self.device.leds.len())
            .map(|i| colors.get(i).copied().unwrap_or_default())
            .collect();
        let changed = self.last.as_deref() != Some(&colors[..]);
        let result = self.ctl.update(&self.zones, &colors, changed);
        self.last = result.is_ok().then_some(colors);
        result
    }
}

/// One device a node reaches: `HIDPP20BuildTarget`.
#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub identity: String,
    pub index: u8,
    pub behind_receiver: bool,
    pub pairing_name: String,
}

/// `HIDPP20Enumerate` for a receiver's pairing table, read over its short (usage 1) collection.
pub fn receiver_targets(rap: &mut impl Link, node_path: &str) -> Vec<Target> {
    let mut targets = Vec::new();
    for (pid, index) in receiver::paired(rap) {
        if receiver::legacy(pid) {
            continue;
        }
        let mut identity = receiver::serial(rap, index);
        let pairing_name = receiver::name(rap, index);
        if identity.is_empty() {
            identity = format!("{node_path}#{index}");
        }
        targets.push(Target {
            identity,
            index,
            behind_receiver: true,
            pairing_name,
        });
    }
    targets
}

/// `HIDPP20BuildController`: probe, and keep it when it has zone effects or per-key lighting.
pub fn build<L: Link>(
    link: L,
    vl: Option<L>,
    target: &Target,
    bluetooth: bool,
) -> Option<Unified<L>> {
    let mut ctl = Hidpp::new(
        link,
        target.index,
        target.behind_receiver,
        bluetooth,
        target.pairing_name.clone(),
    );
    ctl.vl = vl;
    if ctl.probe() != Probed::Lit
        || !(ctl.caps.has_zone_effects || ctl.caps.has_perkey || perkey_8080(&ctl.caps))
    {
        return None;
    }
    Some(Unified::new(ctl, &target.identity))
}

/// The usage 0x0604 collection beside an 0xFF43 node, which carries the 0x12 report on Windows.
fn very_long(api: &HidApi, info: &Info) -> Option<HidDevice> {
    if info.usage_page != 0xFF43 {
        return None;
    }
    let key = receiver::path_key(&info.path);
    crate::device_list(api).iter()
        .find(|d| {
            d.vendor_id() == info.vid
                && d.product_id() == info.pid
                && d.usage_page() == 0xFF43
                && d.usage() == 0x0604
                && receiver::path_key(&d.path().to_string_lossy()) == key
        })
        .and_then(|d| d.open_device(api).ok())
}

/// `DetectLogitechHIDPP20`.
fn open(api: &HidApi, info: &Info, claimed: &mut Vec<String>) -> Vec<Box<dyn Backend>> {
    if info.vid != VID || !on_usb_or_bluetooth(info.bus) {
        return Vec::new();
    }
    let bluetooth = info.bus == Bus::Bluetooth;
    let Ok(hid) = info.open(api) else {
        return Vec::new();
    };
    let identity = Hidpp::new(hid, DEFAULT_INDEX, false, bluetooth, String::new()).probe_identity();
    let targets = if !identity.is_empty() {
        vec![Target {
            identity,
            index: DEFAULT_INDEX,
            behind_receiver: false,
            pairing_name: String::new(),
        }]
    } else if info.usage_page == 0xFF00 {
        let key = receiver::path_key(&info.path);
        let listed = crate::device_list(api);
        let rap = listed.iter().find(|d| {
            d.vendor_id() == info.vid
                && d.product_id() == info.pid
                && d.interface_number() == 2
                && d.usage() == 1
                && receiver::path_key(&d.path().to_string_lossy()) == key
        });
        match rap.map(|d| d.open_device(api)) {
            Some(Ok(mut rap)) => receiver_targets(&mut rap, &info.path),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    for target in targets {
        if claimed.contains(&target.identity) {
            continue;
        }
        claimed.push(target.identity.clone());
        let built = info
            .open(api)
            .ok()
            .and_then(|hid| build(hid, very_long(api, info), &target, bluetooth));
        match built {
            Some(device) => found.push(Box::new(device)),
            None => claimed.retain(|c| *c != target.identity),
        }
    }
    found
}

#[cfg(test)]
#[path = "hidpp20_tests.rs"]
mod tests;
