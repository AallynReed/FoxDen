//! Logitech HID++ 2.0 lighting, behind a Lightspeed receiver or on a cable.

use std::time::{Duration, Instant};

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;

pub const VID: u16 = 0x046D;
pub const G502_RECEIVER: u16 = 0xC539;
pub const G515_RECEIVER: u16 = 0xC547;
pub const G502_WIRED: u16 = 0xC08D;
pub const G515_WIRED: u16 = 0xC355;
/// G915 X LIGHTSPEED on its cable; over the air it pairs with the G515's receiver.
pub const G915X_LS_WIRED: u16 = 0xC356;
/// G915 X, the cable-only model.
pub const G915X_WIRED: u16 = 0xC359;

/// Every product id this module drives.
pub const PIDS: [u16; 6] = [
    G502_RECEIVER,
    G515_RECEIVER,
    G502_WIRED,
    G515_WIRED,
    G915X_LS_WIRED,
    G915X_WIRED,
];

/// A device on its own cable, answering at index 0xFF.
pub fn cable(pid: u16) -> bool {
    matches!(pid, G502_WIRED | G515_WIRED | G915X_LS_WIRED | G915X_WIRED)
}

/// What a receiver stores for its paired device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paired {
    pub wpid: u16,
    pub kind: u8,
    pub name: String,
}

/// Device type 1 in the receiver's pairing info.
const PAIRED_KEYBOARD: u8 = 0x01;
const NO_LIGHTING: &str = "no lighting feature";

/// A Lightspeed receiver has its one paired device at index 1; a device on a cable answers at 0xFF.
const RECEIVER_INDEX: u8 = 0x01;
const WIRED_INDEX: u8 = 0xFF;
const SW_ID: u8 = 0x07;

const COLOR_LED_EFFECTS: u16 = 0x8070;
const RGB_EFFECTS: u16 = 0x8071;
const PER_KEY_V2: u16 = 0x8081;
const ONBOARD_PROFILES: u16 = 0x8100;
const PROFILE_MANAGEMENT: u16 = 0x8101;

/// Where the device's onboard-profile switch lives: the feature index, and whether it is 0x8101.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profiles(pub u8, pub bool);

/// Host mode on: the device stops playing its stored profile and takes our colours.
pub fn host_mode(device: u8, profiles: Profiles, on: bool) -> Report {
    match profiles {
        Profiles(feature, true) => long(device, feature, 0x60, &[if on { 0x05 } else { 0x03 }]),
        Profiles(feature, false) => long(device, feature, 0x10, &[if on { 0x02 } else { 0x01 }]),
    }
}

/// RGB power mode 1 (active) on 0x8071.
pub fn power_on(device: u8, feature: u8) -> Report {
    long(device, feature, 0x80, &[0x01, 0x01])
}

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
    for (slot, &(zone, c)) in p.as_chunks_mut::<4>().0.iter_mut().zip(keys) {
        *slot = [zone, c.0, c.1, c.2];
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
    device: u8,
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
        reports.push(sw_control(device, feature, kind, 3, 6));
    }
    if let Some(feature) = per_key {
        reports.extend(
            changed
                .chunks(4)
                .map(|keys| set_keys(device, feature, keys)),
        );
        reports.push(frame_end(device, feature));
        // the reference moves to flags 5 once the first per-key frame is in place
        if first && let Some((feature, Effects::Rgb)) = effects {
            reports.push(sw_control(device, feature, Effects::Rgb, 3, 5));
        }
    } else if let Some((feature, kind)) = effects {
        reports.extend(
            changed
                .iter()
                .map(|&(zone, c)| set_static(device, feature, kind, zone, c)),
        );
    }
    reports
}

const TKL_WIDTH: f32 = 18.25;

/// TKL rows as (zone id, key width), zone 0 being a gap. Ids are the HID usage minus 3,
/// with the eight modifiers from 104.
const TKL: [&[(u8, f32)]; 6] = [
    // Esc, F1 to F12, Print, Scroll, Pause
    &[
        (38, 1.0),
        (0, 1.0),
        (55, 1.0),
        (56, 1.0),
        (57, 1.0),
        (58, 1.0),
        (0, 0.5),
        (59, 1.0),
        (60, 1.0),
        (61, 1.0),
        (62, 1.0),
        (0, 0.5),
        (63, 1.0),
        (64, 1.0),
        (65, 1.0),
        (66, 1.0),
        (0, 0.25),
        (67, 1.0),
        (68, 1.0),
        (69, 1.0),
    ],
    // grave, 1 to 0, minus, equal, Backspace, Insert, Home, Page Up
    &[
        (50, 1.0),
        (27, 1.0),
        (28, 1.0),
        (29, 1.0),
        (30, 1.0),
        (31, 1.0),
        (32, 1.0),
        (33, 1.0),
        (34, 1.0),
        (35, 1.0),
        (36, 1.0),
        (42, 1.0),
        (43, 1.0),
        (39, 2.0),
        (0, 0.25),
        (70, 1.0),
        (71, 1.0),
        (72, 1.0),
    ],
    // Tab, Q W E R T Y U I O P, brackets, backslash, Delete, End, Page Down
    &[
        (40, 1.5),
        (17, 1.0),
        (23, 1.0),
        (5, 1.0),
        (18, 1.0),
        (20, 1.0),
        (25, 1.0),
        (21, 1.0),
        (9, 1.0),
        (15, 1.0),
        (16, 1.0),
        (44, 1.0),
        (45, 1.0),
        (46, 1.5),
        (0, 0.25),
        (73, 1.0),
        (74, 1.0),
        (75, 1.0),
    ],
    // Caps, A S D F G H J K L, semicolon, quote, Enter
    &[
        (54, 1.75),
        (1, 1.0),
        (19, 1.0),
        (4, 1.0),
        (6, 1.0),
        (7, 1.0),
        (8, 1.0),
        (10, 1.0),
        (11, 1.0),
        (12, 1.0),
        (48, 1.0),
        (49, 1.0),
        (47, 1.0),
        (37, 1.25),
    ],
    // Shift, ISO key, Z X C V B N M, comma, period, slash, Shift, Up
    &[
        (105, 1.25),
        (97, 1.0),
        (26, 1.0),
        (24, 1.0),
        (3, 1.0),
        (22, 1.0),
        (2, 1.0),
        (14, 1.0),
        (13, 1.0),
        (51, 1.0),
        (52, 1.0),
        (53, 1.0),
        (109, 2.75),
        (0, 1.25),
        (79, 1.0),
    ],
    // Ctrl, Win, Alt, Space, Alt, Win or Fn, Menu, Ctrl, Left, Down, Right
    &[
        (104, 1.25),
        (107, 1.25),
        (106, 1.25),
        (41, 6.25),
        (110, 1.25),
        (111, 1.25),
        (98, 1.25),
        (108, 1.25),
        (0, 0.25),
        (77, 1.0),
        (78, 1.0),
        (76, 1.0),
    ],
];

/// Zone id and position (0..=1) of each TKL key, row by row.
pub fn tkl_layout() -> Vec<(u8, [f32; 2])> {
    let mut keys = Vec::new();
    for (row, cells) in TKL.iter().enumerate() {
        let mut x = 0.0;
        for &(zone, width) in *cells {
            if zone != 0 {
                keys.push((
                    zone,
                    [(x + width / 2.0) / TKL_WIDTH, (row as f32 + 0.5) / 6.0],
                ));
            }
            x += width;
        }
    }
    keys
}

const FULL_WIDTH: f32 = 22.5;

/// Number pad rows as (zone id, column, row, width, height); ids follow the TKL rule.
const NUMPAD: [(u8, f32, f32, f32, f32); 17] = [
    (80, 0.0, 1.0, 1.0, 1.0),
    (81, 1.0, 1.0, 1.0, 1.0),
    (82, 2.0, 1.0, 1.0, 1.0),
    (83, 3.0, 1.0, 1.0, 1.0),
    (92, 0.0, 2.0, 1.0, 1.0),
    (93, 1.0, 2.0, 1.0, 1.0),
    (94, 2.0, 2.0, 1.0, 1.0),
    (84, 3.0, 2.0, 1.0, 2.0),
    (89, 0.0, 3.0, 1.0, 1.0),
    (90, 1.0, 3.0, 1.0, 1.0),
    (91, 2.0, 3.0, 1.0, 1.0),
    (86, 0.0, 4.0, 1.0, 1.0),
    (87, 1.0, 4.0, 1.0, 1.0),
    (88, 2.0, 4.0, 1.0, 1.0),
    (85, 3.0, 4.0, 1.0, 2.0),
    (95, 0.0, 5.0, 2.0, 1.0),
    (96, 2.0, 5.0, 1.0, 1.0),
];

/// Zone id and position of each key of a full-size board: the TKL block, then the number pad.
pub fn full_layout() -> Vec<(u8, [f32; 2])> {
    let scale = TKL_WIDTH / FULL_WIDTH;
    let mut keys: Vec<(u8, [f32; 2])> = tkl_layout()
        .into_iter()
        .map(|(zone, [x, y])| (zone, [x * scale, y]))
        .collect();
    for (zone, col, row, w, h) in NUMPAD {
        let x = (TKL_WIDTH + 0.25 + col + w / 2.0) / FULL_WIDTH;
        keys.push((zone, [x, (row + h / 2.0) / 6.0]));
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
    /// HID++ address of the device: 1 behind a receiver, 0xFF on a cable.
    index: u8,
    keyboard: bool,
    effects: Option<(u8, Effects)>,
    per_key: Option<u8>,
    profiles: Option<Profiles>,
    /// Zone id of each LED.
    zones: Vec<u8>,
    /// Zones the keyboard reported; writes to others are skipped.
    reported: Vec<u8>,
    /// Features found; false while the device is asleep or switched off.
    ready: bool,
    tried: Instant,
    /// Colours on the device, empty until we have taken control.
    sent: Vec<Color>,
    /// When the last frame went out.
    last: Instant,
}

/// How often a sleeping device is asked again.
const WAKE_CHECK: Duration = Duration::from_secs(3);

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

fn feature(hid: &HidDevice, device: u8, id: u16) -> Result<Option<u8>, String> {
    let index = request(hid, &get_feature(device, id))?[0];
    Ok((index != 0).then_some(index))
}

/// The receiver product id of a model, wired or not.
pub fn receiver_of(pid: u16) -> u16 {
    match pid {
        G502_WIRED => G502_RECEIVER,
        G515_WIRED | G915X_LS_WIRED => G515_RECEIVER,
        other => other,
    }
}

/// Keyboard or not, and the model name: from the pairing info when the receiver gave it.
pub fn identify(pid: u16, paired: Option<&Paired>) -> (bool, String) {
    let keyboard = match paired {
        Some(p) => p.kind == PAIRED_KEYBOARD,
        None => matches!(
            pid,
            G515_RECEIVER | G515_WIRED | G915X_LS_WIRED | G915X_WIRED
        ),
    };
    let model = match paired {
        Some(p) if !p.name.is_empty() => p.name.clone(),
        _ if pid == G915X_LS_WIRED => "G915 X LS".into(),
        _ if pid == G915X_WIRED => "G915 X".into(),
        _ if keyboard => "G515 Lightspeed TKL".into(),
        _ => "G502 Lightspeed".into(),
    };
    (keyboard, model)
}

impl Lightspeed {
    /// The device behind the receiver. Its features are read now if it is awake, otherwise once it wakes.
    /// `paired` is what the receiver stores for slot 1; without it the receiver's usual model is assumed.
    pub fn open(hid: HidDevice, pid: u16, paired: Option<Paired>) -> Result<Self, String> {
        let wired = cable(pid);
        let index = if wired { WIRED_INDEX } else { RECEIVER_INDEX };
        let (keyboard, model) = identify(pid, paired.as_ref());
        let link = if wired { " (cable)" } else { " (wireless)" };
        let mut light = Lightspeed {
            hid,
            device: Device::grid("", "", DeviceKind::Other, [0.0; 4], 1, 1),
            index,
            keyboard,
            effects: None,
            per_key: None,
            profiles: None,
            zones: Vec::new(),
            reported: Vec::new(),
            ready: false,
            tried: Instant::now(),
            sent: Vec::new(),
            last: Instant::now(),
        };
        if let Err(e) = light.discover()
            && e == NO_LIGHTING
        {
            return Err(format!("Logitech {model}: {e}"));
        }
        // One id per model, so its layout stays the same over the cable and the receiver.
        let id = format!("hid:logitech-{:04x}", receiver_of(pid));
        let name = format!("Logitech {model}{link}");
        // An awake keyboard without per-key lighting is shown as one zone.
        let per_key = keyboard && (!light.ready || light.per_key.is_some());
        (light.zones, light.device) = if per_key {
            let full = model.starts_with("G915");
            let keys = if full { full_layout() } else { tkl_layout() };
            let device = Device {
                id,
                name,
                kind: DeviceKind::Keyboard,
                rect: [0.06, 0.58, 0.54, 0.32],
                grid: [if full { 22 } else { 18 }, 6],
                leds: keys.iter().map(|(_, at)| *at).collect(),
            };
            (keys.iter().map(|(zone, _)| *zone).collect(), device)
        } else if keyboard {
            let rect = [0.06, 0.58, 0.54, 0.32];
            let device = Device::grid(&id, &name, DeviceKind::Keyboard, rect, 1, 1);
            (vec![0], device)
        } else {
            let rect = [0.70, 0.62, 0.06, 0.22];
            let device = Device::grid(&id, &name, DeviceKind::Mouse, rect, 1, 2);
            (vec![0, 1], device)
        };
        Ok(light)
    }

    fn discover(&mut self) -> Result<(), String> {
        self.tried = Instant::now();
        let hid = &self.hid;
        let device = self.index;
        let effects = match feature(hid, device, RGB_EFFECTS)? {
            Some(index) => Some((index, Effects::Rgb)),
            None => {
                feature(hid, device, COLOR_LED_EFFECTS)?.map(|index| (index, Effects::ColorLed))
            }
        };
        let per_key = if self.keyboard {
            feature(hid, device, PER_KEY_V2)?
        } else {
            None
        };
        let profiles = match feature(hid, device, PROFILE_MANAGEMENT)? {
            Some(index) => Some(Profiles(index, true)),
            None => feature(hid, device, ONBOARD_PROFILES)?.map(|index| Profiles(index, false)),
        };
        let mut reported = Vec::new();
        if let Some(index) = per_key {
            for page in 0..3 {
                reported.extend(zones_in_page(
                    page,
                    &request(hid, &zone_page_query(device, index, page))?,
                ));
            }
        }
        if effects.is_none() && per_key.is_none() {
            return Err(NO_LIGHTING.into());
        }
        self.effects = effects;
        self.per_key = per_key.filter(|_| !reported.is_empty());
        self.profiles = profiles;
        self.reported = reported;
        self.ready = true;
        self.sent.clear();
        Ok(())
    }

    /// The device stopped answering: wait for it to wake, then take it over again.
    fn asleep(&mut self) {
        self.ready = false;
        self.sent.clear();
        self.tried = Instant::now();
    }

    /// Software control, RGB power on, then host mode.
    fn claim(&self) -> Result<(), String> {
        if let Some((feature, kind)) = self.effects {
            request(&self.hid, &sw_control(self.index, feature, kind, 3, 6))?;
            if kind == Effects::Rgb {
                let _ = request(&self.hid, &power_on(self.index, feature));
            }
        }
        if let Some(profiles) = self.profiles {
            request(&self.hid, &host_mode(self.index, profiles, true))?;
        }
        Ok(())
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
            return Err(format!(
                "expected {} colours, got {}",
                self.zones.len(),
                colors.len()
            ));
        }
        if !self.ready {
            if self.tried.elapsed() < WAKE_CHECK {
                return Ok(());
            }
            if self.discover().is_err() {
                return Ok(());
            }
        }
        // a frame skipped here is not lost: `sent` stays as it was, so the next push carries the change
        if !due(self.last.elapsed(), colors) {
            return Ok(());
        }
        let (zones, known): (Vec<u8>, Vec<Color>) = if self.reported.is_empty() {
            (self.zones.clone(), colors.to_vec())
        } else {
            self.zones
                .iter()
                .zip(colors)
                .filter(|(zone, _)| self.reported.contains(zone))
                .map(|(zone, c)| (*zone, *c))
                .unzip()
        };
        let reports = plan(
            self.index,
            self.effects,
            self.per_key,
            &zones,
            &self.sent,
            &known,
        );
        if reports.is_empty() {
            return Ok(());
        }
        if self.sent.is_empty() && self.claim().is_err() {
            self.asleep();
            return Ok(());
        }
        // drop queued replies so the receiver's input buffer never backs up
        let mut buf = [0u8; 64];
        while matches!(self.hid.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
        // each report waits for its acknowledgement
        for report in &reports {
            let mut tries = 0;
            loop {
                match request(&self.hid, report) {
                    Ok(_) => break,
                    // busy: the keyboard is still drawing the last frame
                    Err(e) if e.ends_with("0x08") && tries < 50 => {
                        tries += 1;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) if e.contains("no reply") => {
                        self.asleep();
                        return Ok(());
                    }
                    Err(e) => {
                        self.sent.clear();
                        return Err(format!("function {:#04x}: {e}", report[3]));
                    }
                }
            }
        }
        self.sent = known;
        self.last = Instant::now();
        Ok(())
    }
}

impl Drop for Lightspeed {
    fn drop(&mut self) {
        if self.sent.is_empty() {
            return;
        }
        if let Some((feature, kind)) = self.effects {
            let _ = self.hid.write(&sw_control(self.index, feature, kind, 0, 0));
        }
        if let Some(profiles) = self.profiles {
            let _ = self.hid.write(&host_mode(self.index, profiles, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color(255, 0, 0);

    #[test]
    fn long_report_framing() {
        let r = long(0x01, 0x09, 0x10, &[0xAA, 0xBB]);
        assert_eq!(r[..6], [0x11, 0x01, 0x09, 0x17, 0xAA, 0xBB]);
        assert!(r[6..].iter().all(|&b| b == 0));
    }

    // Root feature, function 0, id big-endian.
    #[test]
    fn get_feature_report() {
        let r = get_feature(0x01, 0x8081);
        assert_eq!(r[..6], [0x11, 0x01, 0x00, 0x07, 0x80, 0x81]);
    }

    #[test]
    fn reply_matching() {
        let request = get_feature(0x01, 0x8071);
        let mut reply = [0u8; 20];
        reply[..7].copy_from_slice(&[0x11, 0x01, 0x00, 0x07, 0x0A, 0x00, 0x04]);
        assert_eq!(
            reply_to(&request, &reply).unwrap().unwrap()[..3],
            [0x0A, 0x00, 0x04]
        );
        // HID++ 2.0 error: 0xFF, then the feature index, function and code
        assert_eq!(
            reply_to(&request, &[0x11, 0x01, 0xFF, 0x00, 0x07, 0x05, 0x00]),
            Some(Err(0x05))
        );
        // another device index, another function, a short read
        assert_eq!(
            reply_to(&request, &[0x11, 0x02, 0x00, 0x07, 0x0A, 0x00, 0x04]),
            None
        );
        assert_eq!(
            reply_to(&request, &[0x11, 0x01, 0x00, 0x17, 0x0A, 0x00, 0x04]),
            None
        );
        assert_eq!(reply_to(&request, &[]), None);
    }

    // 0x8070 leaves its persist byte 0.
    #[test]
    fn sw_control_report() {
        assert_eq!(
            sw_control(1, 0x0A, Effects::Rgb, 3, 6)[..7],
            [0x11, 0x01, 0x0A, 0x57, 0x01, 0x03, 0x06]
        );
        assert_eq!(
            sw_control(1, 0x0A, Effects::Rgb, 0, 0)[..7],
            [0x11, 0x01, 0x0A, 0x57, 0x01, 0x00, 0x00]
        );
        assert_eq!(
            sw_control(1, 0x06, Effects::ColorLed, 3, 6)[..6],
            [0x11, 0x01, 0x06, 0x87, 0x01, 0x00]
        );
        assert_eq!(
            sw_control(1, 0x06, Effects::ColorLed, 0, 0)[..6],
            [0x11, 0x01, 0x06, 0x87, 0x00, 0x00]
        );
    }

    // Static effect, persist false.
    #[test]
    fn set_static_report() {
        let r = set_static(1, 0x06, Effects::ColorLed, 1, Color(0x10, 0x20, 0x30));
        assert_eq!(
            r[..10],
            [0x11, 0x01, 0x06, 0x37, 0x01, 0x01, 0x10, 0x20, 0x30, 0x02]
        );
        assert!(r[10..].iter().all(|&b| b == 0));
        assert_eq!(r[16], 0, "persist byte must stay clear");
        let off = set_static(1, 0x0A, Effects::Rgb, 0, Color::BLACK);
        assert_eq!(off[..10], [0x11, 0x01, 0x0A, 0x17, 0x00, 0x01, 0, 0, 0, 0]);
    }

    #[test]
    fn per_key_reports() {
        let r = set_keys(1, 0x0B, &[(38, RED), (1, Color(1, 2, 3))]);
        assert_eq!(r[..12], [0x11, 0x01, 0x0B, 0x17, 38, 255, 0, 0, 1, 1, 2, 3]);
        assert!(r[12..].iter().all(|&b| b == 0));
        let end = frame_end(1, 0x0B);
        assert_eq!(end[..4], [0x11, 0x01, 0x0B, 0x77]);
        assert!(end[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn zone_bitmap() {
        assert_eq!(
            zone_page_query(1, 0x0B, 2)[..7],
            [0x11, 0x01, 0x0B, 0x07, 0x00, 0x02, 0x00]
        );
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
        assert!(
            plan(
                RECEIVER_INDEX,
                Some((0x0A, Effects::Rgb)),
                Some(0x0B),
                &zones,
                &frame,
                &frame
            )
            .is_empty()
        );
        assert!(
            plan(
                RECEIVER_INDEX,
                Some((0x06, Effects::ColorLed)),
                None,
                &[0, 1],
                &frame[..2],
                &frame[..2]
            )
            .is_empty()
        );
    }

    #[test]
    fn plan_first_keyboard_frame_claims_then_paints_every_key() {
        let zones: Vec<u8> = (1..=9).collect();
        let reports = plan(
            RECEIVER_INDEX,
            Some((0x0A, Effects::Rgb)),
            Some(0x0B),
            &zones,
            &[],
            &[RED; 9],
        );
        let functions: Vec<[u8; 2]> = reports.iter().map(|r| [r[2], r[3]]).collect();
        assert_eq!(
            functions,
            [
                [0x0A, 0x57],
                [0x0B, 0x17],
                [0x0B, 0x17],
                [0x0B, 0x17],
                [0x0B, 0x77],
                [0x0A, 0x57]
            ]
        );
        assert_eq!(reports[0][4..7], [0x01, 0x03, 0x06]);
        assert_eq!(reports[3][4..12], [9, 255, 0, 0, 0, 0, 0, 0]);
        assert_eq!(reports[5][4..7], [0x01, 0x03, 0x05]);
    }

    #[test]
    fn plan_sends_only_changed_keys() {
        let zones = [1, 2, 3, 4, 5];
        let mut next = [RED; 5];
        next[3] = Color(0, 0, 9);
        let reports = plan(
            RECEIVER_INDEX,
            Some((0x0A, Effects::Rgb)),
            Some(0x0B),
            &zones,
            &[RED; 5],
            &next,
        );
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0][3..8], [0x17, 4, 0, 0, 9]);
        assert_eq!(reports[1][3], 0x77);
    }

    #[test]
    fn plan_mouse_zones_use_static_effect() {
        let effects = Some((0x06, Effects::ColorLed));
        let first = plan(RECEIVER_INDEX, effects, None, &[0, 1], &[], &[RED, RED]);
        assert_eq!(first.len(), 3);
        assert_eq!(first[0][2..6], [0x06, 0x87, 0x01, 0x00]);
        assert_eq!(first[2][2..9], [0x06, 0x37, 0x01, 0x01, 255, 0, 0]);
        let next = plan(
            RECEIVER_INDEX,
            effects,
            None,
            &[0, 1],
            &[RED, RED],
            &[RED, Color(0, 255, 0)],
        );
        assert_eq!(next.len(), 1);
        assert_eq!(next[0][4], 1);
    }

    #[test]
    fn tkl_layout_has_89_distinct_keys_inside_the_device() {
        let keys = tkl_layout();
        assert_eq!(keys.len(), 89);
        let mut zones: Vec<u8> = keys.iter().map(|(zone, _)| *zone).collect();
        zones.sort_unstable();
        zones.dedup();
        assert_eq!(zones.len(), 89);
        assert!(
            keys.iter()
                .flat_map(|(_, at)| at)
                .all(|v| (0.0..=1.0).contains(v))
        );
        for cells in TKL {
            let width: f32 = cells.iter().map(|(_, w)| w).sum();
            assert!(width <= TKL_WIDTH);
        }
    }

    #[test]
    fn full_layout_adds_the_number_pad() {
        let keys = full_layout();
        assert_eq!(keys.len(), 89 + 17);
        let mut zones: Vec<u8> = keys.iter().map(|(zone, _)| *zone).collect();
        zones.sort_unstable();
        zones.dedup();
        assert_eq!(zones.len(), keys.len());
        assert!((80..=96).all(|z| zones.contains(&z)));
        assert!(
            keys.iter()
                .flat_map(|(_, at)| at)
                .all(|v| (0.0..=1.0).contains(v))
        );
    }

    // G915 X LS on its cable: the 0x8081 zone bitmap pages it answered.
    #[test]
    fn g915x_zone_pages_from_capture() {
        let page = |s: &str| -> [u8; 16] {
            let mut p = [0u8; 16];
            for (i, b) in p.iter_mut().enumerate() {
                *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
            }
            p
        };
        let first = zones_in_page(0, &page("0000ffffffffffffffffffffffffffff"));
        assert_eq!(first, (1..=111).collect::<Vec<u8>>());
        let second = zones_in_page(1, &page("000100000000007a0000f01f00000400"));
        assert_eq!(
            second,
            [
                153, 155, 156, 157, 158, 180, 181, 182, 183, 184, 185, 186, 187, 188, 210
            ]
        );
        assert_eq!(
            zones_in_page(2, &page("00020000000000000000000000000000")),
            Vec::<u8>::new()
        );
        let keys = full_layout();
        assert!(keys.iter().all(|(zone, _)| first.contains(zone)));
    }

    #[test]
    fn g915x_pids_reuse_the_g515_receiver() {
        assert_eq!(receiver_of(G915X_LS_WIRED), G515_RECEIVER);
        assert_eq!(receiver_of(G915X_WIRED), G915X_WIRED);
        assert!(cable(G915X_WIRED) && cable(G915X_LS_WIRED) && !cable(G515_RECEIVER));
        assert_eq!(identify(G915X_LS_WIRED, None), (true, "G915 X LS".into()));
        let paired = Paired {
            wpid: 0,
            kind: 1,
            name: "G915 X LS".into(),
        };
        assert_eq!(
            identify(G515_RECEIVER, Some(&paired)),
            (true, "G915 X LS".into())
        );
    }

    #[test]
    fn frames_are_paced_but_black_always_goes_out() {
        assert!(due(FRAME_GAP, &[RED]));
        assert!(!due(Duration::from_millis(5), &[RED, Color::BLACK]));
        assert!(due(Duration::ZERO, &[Color::BLACK; 3]));
    }

    #[test]
    fn receiver_pairing_decides_the_model() {
        let g502x = Paired {
            wpid: 0x409F,
            kind: 2,
            name: "G502 X LS".into(),
        };
        assert_eq!(
            identify(G515_RECEIVER, Some(&g502x)),
            (false, "G502 X LS".into())
        );
        let keyboard = Paired {
            kind: 1,
            name: String::new(),
            ..g502x
        };
        assert_eq!(
            identify(G502_RECEIVER, Some(&keyboard)),
            (true, "G515 Lightspeed TKL".into())
        );
        assert_eq!(
            identify(G515_RECEIVER, None),
            (true, "G515 Lightspeed TKL".into())
        );
        assert_eq!(
            identify(G502_WIRED, None),
            (false, "G502 Lightspeed".into())
        );
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
