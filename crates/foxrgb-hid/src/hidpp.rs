//! Logitech HID++ 2.0 lighting for devices behind a Lightspeed receiver.

use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;

pub const VID: u16 = 0x046D;
pub const G502_RECEIVER: u16 = 0xC539;
pub const G515_RECEIVER: u16 = 0xC547;

/// A Lightspeed receiver has its one paired device at index 1.
const DEVICE: u8 = 0x01;
const SW_ID: u8 = 0x07;

const COLOR_LED_EFFECTS: u16 = 0x8070;
const RGB_EFFECTS: u16 = 0x8071;
const PER_KEY_V2: u16 = 0x8081;

pub type Report = [u8; 20];

/// The zone effect feature a device has; the two differ in function numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effects {
    ColorLed,
    Rgb,
}

/// True for the receiver collection that carries long HID++ reports.
pub fn is_long_channel(usage_page: u16, usage: u16) -> bool {
    usage_page == 0xFF00 && usage == 0x0002
}

pub fn long(device: u8, feature: u8, function: u8, params: &[u8]) -> Report {
    let mut r = [0u8; 20];
    r[0] = 0x11;
    r[1] = device;
    r[2] = feature;
    r[3] = function | SW_ID;
    r[4..4 + params.len()].copy_from_slice(params);
    r
}

/// Asks the root feature where `id` lives; the reply's first parameter is its index, 0 if absent.
pub fn get_feature(device: u8, id: u16) -> Report {
    long(device, 0x00, 0x00, &id.to_be_bytes())
}

/// Parameters of the reply to `request`, or the device's error code. `None` for unrelated reports.
pub fn reply_to(request: &Report, report: &[u8]) -> Option<Result<[u8; 16], u8>> {
    if report.len() < 7 || report[1] != request[1] {
        return None;
    }
    if matches!(report[2], 0xFF | 0x8F) && report[3..5] == request[2..4] {
        return Some(Err(report[5]));
    }
    if report[2..4] != request[2..4] {
        return None;
    }
    let mut params = [0u8; 16];
    let n = (report.len() - 4).min(16);
    params[..n].copy_from_slice(&report[4..4 + n]);
    Some(Ok(params))
}

/// Takes or returns control of the lighting. Nothing here is stored on the device.
pub fn sw_control(device: u8, feature: u8, kind: Effects, mode: u8, flags: u8) -> Report {
    match kind {
        Effects::ColorLed => long(device, feature, 0x80, &[(mode > 0) as u8, 0x00]),
        Effects::Rgb => long(device, feature, 0x50, &[0x01, mode, flags]),
    }
}

/// Fixed colour on one zone. Parameter 12 is the persist byte and stays 0.
pub fn set_static(device: u8, feature: u8, kind: Effects, zone: u8, c: Color) -> Report {
    let mut p = [0u8; 13];
    p[0] = zone;
    p[1] = 0x01;
    p[2..5].copy_from_slice(&[c.0, c.1, c.2]);
    if c != Color::BLACK {
        p[5] = 0x02;
    }
    let function = match kind {
        Effects::ColorLed => 0x30,
        Effects::Rgb => 0x10,
    };
    long(device, feature, function, &p)
}

/// Up to four keys per report.
pub fn set_keys(device: u8, feature: u8, keys: &[(u8, Color)]) -> Report {
    let mut p = [0u8; 16];
    for (slot, &(zone, c)) in p.chunks_exact_mut(4).zip(keys) {
        slot.copy_from_slice(&[zone, c.0, c.1, c.2]);
    }
    long(device, feature, 0x10, &p)
}

pub fn frame_end(device: u8, feature: u8) -> Report {
    long(device, feature, 0x70, &[])
}

/// Requests one of the three 112-bit pages of the per-key zone bitmap.
pub fn zone_page_query(device: u8, feature: u8, page: u8) -> Report {
    long(device, feature, 0x00, &[0x00, page, 0x00])
}

/// Zone ids set in a bitmap page reply. Zone 0 is unused and ids above 255 cannot be addressed.
pub fn zones_in_page(page: u8, params: &[u8; 16]) -> Vec<u8> {
    (0..112u16)
        .filter(|bit| params[2 + (bit / 8) as usize] & (1 << (bit % 8)) != 0)
        .map(|bit| page as u16 * 112 + bit)
        .filter(|&zone| zone != 0)
        .filter_map(|zone| u8::try_from(zone).ok())
        .collect()
}

/// Reports that take a device from `sent` to `colors`; `sent` is empty before the first frame.
pub fn plan(
    effects: Option<(u8, Effects)>,
    per_key: Option<u8>,
    zones: &[u8],
    sent: &[Color],
    colors: &[Color],
) -> Vec<Report> {
    let first = sent.is_empty();
    let changed: Vec<(u8, Color)> = zones
        .iter()
        .zip(colors)
        .enumerate()
        .filter(|(i, _)| first || sent[*i] != colors[*i])
        .map(|(_, (zone, c))| (*zone, *c))
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let mut reports = Vec::new();
    if first && let Some((feature, kind)) = effects {
        reports.push(sw_control(DEVICE, feature, kind, 3, 6));
    }
    if let Some(feature) = per_key {
        reports.extend(changed.chunks(4).map(|keys| set_keys(DEVICE, feature, keys)));
        reports.push(frame_end(DEVICE, feature));
        // the reference moves to flags 5 once the first per-key frame is in place
        if first && let Some((feature, Effects::Rgb)) = effects {
            reports.push(sw_control(DEVICE, feature, Effects::Rgb, 3, 5));
        }
    } else if let Some((feature, kind)) = effects {
        reports.extend(changed.iter().map(|&(zone, c)| set_static(DEVICE, feature, kind, zone, c)));
    }
    reports
}

const TKL_WIDTH: f32 = 18.25;

/// TKL rows as (zone id, key width), zone 0 being a gap. Ids are the HID usage minus 3,
/// with the eight modifiers from 104.
const TKL: [&[(u8, f32)]; 6] = [
    // Esc, F1 to F12, Print, Scroll, Pause
    &[
        (38, 1.0), (0, 1.0), (55, 1.0), (56, 1.0), (57, 1.0), (58, 1.0), (0, 0.5), (59, 1.0), (60, 1.0),
        (61, 1.0), (62, 1.0), (0, 0.5), (63, 1.0), (64, 1.0), (65, 1.0), (66, 1.0), (0, 0.25), (67, 1.0),
        (68, 1.0), (69, 1.0),
    ],
    // grave, 1 to 0, minus, equal, Backspace, Insert, Home, Page Up
    &[
        (50, 1.0), (27, 1.0), (28, 1.0), (29, 1.0), (30, 1.0), (31, 1.0), (32, 1.0), (33, 1.0), (34, 1.0),
        (35, 1.0), (36, 1.0), (42, 1.0), (43, 1.0), (39, 2.0), (0, 0.25), (70, 1.0), (71, 1.0), (72, 1.0),
    ],
    // Tab, Q W E R T Y U I O P, brackets, backslash, Delete, End, Page Down
    &[
        (40, 1.5), (17, 1.0), (23, 1.0), (5, 1.0), (18, 1.0), (20, 1.0), (25, 1.0), (21, 1.0), (9, 1.0),
        (15, 1.0), (16, 1.0), (44, 1.0), (45, 1.0), (46, 1.5), (0, 0.25), (73, 1.0), (74, 1.0), (75, 1.0),
    ],
    // Caps, A S D F G H J K L, semicolon, quote, Enter
    &[
        (54, 1.75), (1, 1.0), (19, 1.0), (4, 1.0), (6, 1.0), (7, 1.0), (8, 1.0), (10, 1.0), (11, 1.0),
        (12, 1.0), (48, 1.0), (49, 1.0), (37, 2.25),
    ],
    // Shift, Z X C V B N M, comma, period, slash, Shift, Up
    &[
        (105, 2.25), (26, 1.0), (24, 1.0), (3, 1.0), (22, 1.0), (2, 1.0), (14, 1.0), (13, 1.0), (51, 1.0),
        (52, 1.0), (53, 1.0), (109, 2.75), (0, 1.25), (79, 1.0),
    ],
    // Ctrl, Win, Alt, Space, Alt, Win or Fn, Menu, Ctrl, Left, Down, Right
    &[
        (104, 1.25), (107, 1.25), (106, 1.25), (41, 6.25), (110, 1.25), (111, 1.25), (98, 1.25),
        (108, 1.25), (0, 0.25), (77, 1.0), (78, 1.0), (76, 1.0),
    ],
];

/// Zone id and position (0..=1) of each TKL key, row by row.
pub fn tkl_layout() -> Vec<(u8, [f32; 2])> {
    let mut keys = Vec::new();
    for (row, cells) in TKL.iter().enumerate() {
        let mut x = 0.0;
        for &(zone, width) in *cells {
            if zone != 0 {
                keys.push((zone, [(x + width / 2.0) / TKL_WIDTH, (row as f32 + 0.5) / 6.0]));
            }
            x += width;
        }
    }
    keys
}

/// LEDs of a per-key keyboard: the reported zones the TKL layout knows, then any others at the centre.
pub fn key_leds(reported: &[u8]) -> Vec<(u8, [f32; 2])> {
    let layout = tkl_layout();
    let mut keys: Vec<_> = layout.iter().copied().filter(|(zone, _)| reported.contains(zone)).collect();
    if !keys.is_empty() {
        let extra = reported.iter().filter(|zone| !layout.iter().any(|(known, _)| known == *zone));
        keys.extend(extra.map(|zone| (*zone, [0.5, 0.5])));
    }
    keys
}

/// Shortest gap between two frames, so a busy effect cannot flood the wireless link.
const FRAME_GAP: Duration = Duration::from_millis(33);

/// Whether a frame goes out now. Black always does: it is how the engine switches a device off.
fn due(since_last: Duration, colors: &[Color]) -> bool {
    since_last >= FRAME_GAP || colors.iter().all(|c| *c == Color::BLACK)
}

pub struct Lightspeed {
    hid: HidDevice,
    device: Device,
    effects: Option<(u8, Effects)>,
    per_key: Option<u8>,
    /// Zone id of each LED.
    zones: Vec<u8>,
    /// Colours on the device, empty until we have taken control.
    sent: Vec<Color>,
    /// When the last frame went out.
    last: Instant,
}

fn request(hid: &HidDevice, report: &Report) -> Result<[u8; 16], String> {
    hid.write(report).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        let n = hid.read_timeout(&mut buf, 50).map_err(|e| e.to_string())?;
        if let Some(reply) = reply_to(report, &buf[..n]) {
            return reply.map_err(|code| format!("HID++ error {code:#04x}"));
        }
    }
    Err("no reply from the device".into())
}

fn feature(hid: &HidDevice, id: u16) -> Result<Option<u8>, String> {
    let index = request(hid, &get_feature(DEVICE, id))?[0];
    Ok((index != 0).then_some(index))
}

impl Lightspeed {
    /// Finds the lighting features of the device behind the receiver. Fails if it is asleep or has none.
    pub fn open(hid: HidDevice, receiver: u16) -> Result<Self, String> {
        let keyboard = receiver == G515_RECEIVER;
        let effects = match feature(&hid, RGB_EFFECTS)? {
            Some(index) => Some((index, Effects::Rgb)),
            None => feature(&hid, COLOR_LED_EFFECTS)?.map(|index| (index, Effects::ColorLed)),
        };
        let per_key = if keyboard { feature(&hid, PER_KEY_V2)? } else { None };

        let (id, name, kind, rect) = if keyboard {
            ("hid:logitech-c547", "Logitech G515 Lightspeed TKL", DeviceKind::Keyboard, [0.06, 0.58, 0.54, 0.32])
        } else {
            ("hid:logitech-c539", "Logitech G502 Lightspeed", DeviceKind::Mouse, [0.70, 0.62, 0.06, 0.22])
        };

        let mut reported = Vec::new();
        if let Some(index) = per_key {
            for page in 0..3 {
                if let Ok(params) = request(&hid, &zone_page_query(DEVICE, index, page)) {
                    reported.extend(zones_in_page(page, &params));
                }
            }
        }
        // without a zone list the keyboard is driven as a single zone
        let per_key = per_key.filter(|_| !reported.is_empty());
        let keys = key_leds(&reported);

        let (zones, device) = if !keys.is_empty() {
            let device = Device {
                id: id.into(),
                name: name.into(),
                kind,
                rect,
                grid: [18, 6],
                leds: keys.iter().map(|(_, at)| *at).collect(),
            };
            (keys.iter().map(|(zone, _)| *zone).collect(), device)
        } else if !reported.is_empty() {
            let device = Device::grid(id, name, kind, rect, reported.len() as u32, 1);
            (reported, device)
        } else if effects.is_some() {
            let zones: Vec<u8> = if keyboard { vec![0] } else { vec![0, 1] };
            let device = Device::grid(id, name, kind, rect, 1, zones.len() as u32);
            (zones, device)
        } else {
            return Err("no lighting feature".into());
        };
        Ok(Lightspeed { hid, device, effects, per_key, zones, sent: Vec::new(), last: Instant::now() })
    }
}

impl Backend for Lightspeed {
    fn name(&self) -> &str {
        "logitech hid++"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if colors.len() != self.zones.len() {
            return Err(format!("expected {} colours, got {}", self.zones.len(), colors.len()));
        }
        // a frame skipped here is not lost: `sent` stays as it was, so the next push carries the change
        if !due(self.last.elapsed(), colors) {
            return Ok(());
        }
        let reports = plan(self.effects, self.per_key, &self.zones, &self.sent, colors);
        if reports.is_empty() {
            return Ok(());
        }
        // drop queued replies so the receiver's input buffer never backs up
        let mut buf = [0u8; 64];
        while matches!(self.hid.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
        for report in &reports {
            if let Err(e) = self.hid.write(report) {
                self.sent.clear();
                return Err(e.to_string());
            }
        }
        self.sent = colors.to_vec();
        self.last = Instant::now();
        Ok(())
    }
}

impl Drop for Lightspeed {
    fn drop(&mut self) {
        if !self.sent.is_empty() && let Some((feature, kind)) = self.effects {
            let _ = self.hid.write(&sw_control(DEVICE, feature, kind, 0, 0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color(255, 0, 0);

    // Framing from SendStandard in OpenRGB's LogitechHIDPP20Controller.cpp.
    #[test]
    fn long_report_framing_matches_openrgb_send_standard() {
        let r = long(0x01, 0x09, 0x10, &[0xAA, 0xBB]);
        assert_eq!(r[..6], [0x11, 0x01, 0x09, 0x17, 0xAA, 0xBB]);
        assert!(r[6..].iter().all(|&b| b == 0));
    }

    // GetFeatureIndex in LogitechHIDPP20Controller.cpp: root feature, function 0, id big-endian.
    #[test]
    fn get_feature_matches_openrgb_get_feature_index() {
        let r = get_feature(0x01, 0x8081);
        assert_eq!(r[..6], [0x11, 0x01, 0x00, 0x07, 0x80, 0x81]);
    }

    #[test]
    fn reply_matching() {
        let request = get_feature(0x01, 0x8071);
        let mut reply = [0u8; 20];
        reply[..7].copy_from_slice(&[0x11, 0x01, 0x00, 0x07, 0x0A, 0x00, 0x04]);
        assert_eq!(reply_to(&request, &reply).unwrap().unwrap()[..3], [0x0A, 0x00, 0x04]);
        // HID++ 2.0 error: 0xFF, then the feature index, function and code
        assert_eq!(reply_to(&request, &[0x11, 0x01, 0xFF, 0x00, 0x07, 0x05, 0x00]), Some(Err(0x05)));
        // another device index, another function, a short read
        assert_eq!(reply_to(&request, &[0x11, 0x02, 0x00, 0x07, 0x0A, 0x00, 0x04]), None);
        assert_eq!(reply_to(&request, &[0x11, 0x01, 0x00, 0x17, 0x0A, 0x00, 0x04]), None);
        assert_eq!(reply_to(&request, &[]), None);
    }

    // SetSWControl in LogitechHIDPP20Controller.cpp. We differ on 0x8070 by leaving its persist byte 0.
    #[test]
    fn sw_control_matches_openrgb_set_sw_control() {
        assert_eq!(sw_control(1, 0x0A, Effects::Rgb, 3, 6)[..7], [0x11, 0x01, 0x0A, 0x57, 0x01, 0x03, 0x06]);
        assert_eq!(sw_control(1, 0x0A, Effects::Rgb, 0, 0)[..7], [0x11, 0x01, 0x0A, 0x57, 0x01, 0x00, 0x00]);
        assert_eq!(sw_control(1, 0x06, Effects::ColorLed, 3, 6)[..6], [0x11, 0x01, 0x06, 0x87, 0x01, 0x00]);
        assert_eq!(sw_control(1, 0x06, Effects::ColorLed, 0, 0)[..6], [0x11, 0x01, 0x06, 0x87, 0x00, 0x00]);
    }

    // SetZoneEffect in LogitechHIDPP20Controller.cpp, static effect, persist false.
    #[test]
    fn set_static_matches_openrgb_set_zone_effect() {
        let r = set_static(1, 0x06, Effects::ColorLed, 1, Color(0x10, 0x20, 0x30));
        assert_eq!(r[..10], [0x11, 0x01, 0x06, 0x37, 0x01, 0x01, 0x10, 0x20, 0x30, 0x02]);
        assert!(r[10..].iter().all(|&b| b == 0));
        assert_eq!(r[16], 0, "persist byte must stay clear");
        let off = set_static(1, 0x0A, Effects::Rgb, 0, Color::BLACK);
        assert_eq!(off[..10], [0x11, 0x01, 0x0A, 0x17, 0x00, 0x01, 0, 0, 0, 0]);
    }

    // SetPerKeyColors and PerKeyFrameEnd in LogitechHIDPP20Controller.cpp.
    #[test]
    fn per_key_reports_match_openrgb_set_per_key_colors() {
        let r = set_keys(1, 0x0B, &[(38, RED), (1, Color(1, 2, 3))]);
        assert_eq!(r[..12], [0x11, 0x01, 0x0B, 0x17, 38, 255, 0, 0, 1, 1, 2, 3]);
        assert!(r[12..].iter().all(|&b| b == 0));
        let end = frame_end(1, 0x0B);
        assert_eq!(end[..4], [0x11, 0x01, 0x0B, 0x77]);
        assert!(end[4..].iter().all(|&b| b == 0));
    }

    // DiscoverPerKeyZones in LogitechHIDPP20Controller.cpp.
    #[test]
    fn zone_bitmap_matches_openrgb_discover_per_key_zones() {
        assert_eq!(zone_page_query(1, 0x0B, 2)[..7], [0x11, 0x01, 0x0B, 0x07, 0x00, 0x02, 0x00]);
        let mut params = [0u8; 16];
        params[2] = 0b0000_0111;
        params[3] = 0b1000_0000;
        assert_eq!(zones_in_page(0, &params), [1, 2, 15]);
        assert_eq!(zones_in_page(1, &params), [112, 113, 114, 127]);
        // page 2 starts at 224, so only its first 32 ids fit in a byte
        params = [0u8; 16];
        params[5] = 0b1000_0000;
        params[6] = 0b0000_0001;
        assert_eq!(zones_in_page(2, &params), [255]);
    }

    #[test]
    fn plan_skips_identical_frames() {
        let zones = [1, 2, 3];
        let frame = [RED; 3];
        assert!(plan(Some((0x0A, Effects::Rgb)), Some(0x0B), &zones, &frame, &frame).is_empty());
        assert!(plan(Some((0x06, Effects::ColorLed)), None, &[0, 1], &frame[..2], &frame[..2]).is_empty());
    }

    #[test]
    fn plan_first_keyboard_frame_claims_then_paints_every_key() {
        let zones: Vec<u8> = (1..=9).collect();
        let reports = plan(Some((0x0A, Effects::Rgb)), Some(0x0B), &zones, &[], &[RED; 9]);
        let functions: Vec<[u8; 2]> = reports.iter().map(|r| [r[2], r[3]]).collect();
        assert_eq!(functions, [[0x0A, 0x57], [0x0B, 0x17], [0x0B, 0x17], [0x0B, 0x17], [0x0B, 0x77], [0x0A, 0x57]]);
        assert_eq!(reports[0][4..7], [0x01, 0x03, 0x06]);
        assert_eq!(reports[3][4..12], [9, 255, 0, 0, 0, 0, 0, 0]);
        assert_eq!(reports[5][4..7], [0x01, 0x03, 0x05]);
    }

    #[test]
    fn plan_sends_only_changed_keys() {
        let zones = [1, 2, 3, 4, 5];
        let mut next = [RED; 5];
        next[3] = Color(0, 0, 9);
        let reports = plan(Some((0x0A, Effects::Rgb)), Some(0x0B), &zones, &[RED; 5], &next);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0][3..8], [0x17, 4, 0, 0, 9]);
        assert_eq!(reports[1][3], 0x77);
    }

    #[test]
    fn plan_mouse_zones_use_static_effect() {
        let effects = Some((0x06, Effects::ColorLed));
        let first = plan(effects, None, &[0, 1], &[], &[RED, RED]);
        assert_eq!(first.len(), 3);
        assert_eq!(first[0][2..6], [0x06, 0x87, 0x01, 0x00]);
        assert_eq!(first[2][2..9], [0x06, 0x37, 0x01, 0x01, 255, 0, 0]);
        let next = plan(effects, None, &[0, 1], &[RED, RED], &[RED, Color(0, 255, 0)]);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0][4], 1);
    }

    #[test]
    fn tkl_layout_has_87_distinct_keys_inside_the_device() {
        let keys = tkl_layout();
        assert_eq!(keys.len(), 87);
        let mut zones: Vec<u8> = keys.iter().map(|(zone, _)| *zone).collect();
        zones.sort_unstable();
        zones.dedup();
        assert_eq!(zones.len(), 87);
        assert!(keys.iter().flat_map(|(_, at)| at).all(|v| (0.0..=1.0).contains(v)));
        for cells in TKL {
            let width: f32 = cells.iter().map(|(_, w)| w).sum();
            assert!(width <= TKL_WIDTH);
        }
    }

    #[test]
    fn key_leds_keep_zones_outside_the_layout() {
        // 47 and 97 are the two keys an ISO board adds
        let leds = key_leds(&[1, 38, 47, 97]);
        let zones: Vec<u8> = leds.iter().map(|(zone, _)| *zone).collect();
        assert_eq!(zones, [38, 1, 47, 97]);
        assert_eq!(leds[2].1, [0.5, 0.5]);
        assert_eq!(key_leds(&(1..=111).collect::<Vec<u8>>()).len(), 111);
        // nothing from the layout: the caller falls back to one row
        assert!(key_leds(&[200, 201]).is_empty());
    }

    #[test]
    fn frames_are_paced_but_black_always_goes_out() {
        assert!(due(FRAME_GAP, &[RED]));
        assert!(!due(Duration::from_millis(5), &[RED, Color::BLACK]));
        assert!(due(Duration::ZERO, &[Color::BLACK; 3]));
    }

    #[test]
    fn long_channel_selection() {
        assert!(is_long_channel(0xFF00, 0x0002));
        assert!(!is_long_channel(0xFF00, 0x0001));
        assert!(!is_long_channel(0xFF00, 0x0004));
        assert!(!is_long_channel(0xFFBC, 0x0088));
        assert!(!is_long_channel(0x0001, 0x0002));
    }
}
