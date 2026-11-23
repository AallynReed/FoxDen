//! Logitech HID++ 2.0 lighting, behind a receiver or on a cable.

use std::time::{Duration, Instant};

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;

pub const VID: u16 = 0x046D;
pub const G502_RECEIVER: u16 = 0xC539;
pub const G515_RECEIVER: u16 = 0xC547;
pub const G502_WIRED: u16 = 0xC08D;
pub const G515_WIRED: u16 = 0xC355;
/// G915 X LIGHTSPEED on its cable; over the air it keeps this id.
pub const G915X_LS_WIRED: u16 = 0xC356;
/// G915 X, the cable-only model.
pub const G915X_WIRED: u16 = 0xC359;

/// Product ids with fixed layouts and ids; any other HID++ 2.0 device is driven from what it reports.
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

/// Device types 1 and 2 in the receiver's pairing info.
const PAIRED_KEYBOARD: u8 = 0x01;
const PAIRED_MOUSE: u8 = 0x02;
const NO_LIGHTING: &str = "no lighting feature";

/// A receiver's devices answer at their slot, 1 to 6; a device on a cable answers at 0xFF.
#[cfg(test)]
const RECEIVER_INDEX: u8 = 0x01;
const WIRED_INDEX: u8 = 0xFF;
const SLOTS: u8 = 6;
const SW_ID: u8 = 0x07;

const DEVICE_NAME_TYPE: u16 = 0x0005;
const COLOR_LED_EFFECTS: u16 = 0x8070;
const RGB_EFFECTS: u16 = 0x8071;
const PER_KEY_V1: u16 = 0x8080;
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
    /// HID++ address of the device: its slot behind a receiver, 0xFF on a cable.
    index: u8,
    /// Ask for per-key lighting.
    keys: bool,
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
        G515_WIRED => G515_RECEIVER,
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

/// One device an HID++ collection reaches: a receiver slot, or the device itself on its cable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub pid: u16,
    pub index: u8,
    pub paired: Option<Paired>,
    /// The original G502 and G515 and their cables: fixed layouts, ids kept from before.
    pub known: bool,
    /// What Windows calls the collection's device.
    pub product: String,
}

impl Target {
    /// One id per model, so a layout stays the same across ports and restarts.
    pub fn id(&self) -> String {
        let model = match &self.paired {
            Some(p) if is_g915x(&p.name) => G915X_LS_WIRED,
            _ if self.known => receiver_of(self.pid),
            _ if self.index == WIRED_INDEX => self.pid,
            Some(p) if p.wpid != 0 => p.wpid,
            _ => return format!("hid:logitech-{:04x}-{}", self.pid, self.index),
        };
        format!("hid:logitech-{model:04x}")
    }

    fn model(&self) -> String {
        if let Some(p) = &self.paired
            && !p.name.is_empty()
        {
            return p.name.clone();
        }
        let product = self.product.trim();
        let product = product.strip_prefix("Logitech").unwrap_or(product).trim();
        match self.paired.as_ref().map(|p| p.wpid) {
            _ if !product.is_empty() && !product.eq_ignore_ascii_case("USB Receiver") => {
                product.to_string()
            }
            Some(wpid) if wpid != 0 => format!("device {wpid:04x}"),
            _ => format!("device {:04x}", self.pid),
        }
    }
}

fn is_g915x(name: &str) -> bool {
    name.starts_with("G915 X")
}

/// What a receiver says about one pairing slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Wireless product id and device type.
    Paired(u16, u8),
    Empty,
    /// No answer, or an HID++ 2.0 one: not a receiver.
    Silent,
}

/// Long register 0xB5, pairing info of slot `n`, over the receiver's short collection.
pub fn slot_request(n: u8) -> [u8; 7] {
    [0x10, 0xFF, 0x83, 0xB5, 0x20 + n - 1, 0, 0]
}

/// The answer in `frame` to `slot_request(n)`; `None` for unrelated reports.
pub fn slot_reply(n: u8, frame: &[u8]) -> Option<Slot> {
    let sub = 0x20 + n - 1;
    match frame {
        [_, 0xFF, 0x83, 0xB5, s, ..] if *s == sub && frame.len() >= 12 => {
            let (wpid, kind) = crate::generic_any::receiver::parse_kind(frame);
            Some(if wpid == 0 {
                Slot::Empty
            } else {
                Slot::Paired(wpid, kind)
            })
        }
        [_, 0xFF, 0x8F, 0x83, 0xB5, ..] => Some(Slot::Empty),
        [_, 0xFF, 0xFF, 0x83, ..] => Some(Slot::Silent),
        _ => None,
    }
}

/// Name the receiver stores for slot `n`, from its answer in `frame` to `name_request(n)`.
pub fn name_reply(n: u8, frame: &[u8]) -> Option<String> {
    let sub = 0x40 + n - 1;
    match frame {
        [_, 0xFF, 0x83, 0xB5, s, len, rest @ ..] if *s == sub => {
            let mut name = rest.get(..*len as usize).unwrap_or_default().to_vec();
            while matches!(name.last(), Some(0 | b' ')) {
                name.pop();
            }
            let printable = name.iter().all(|c| (0x20..=0x7E).contains(c));
            Some(if printable && *len <= 14 {
                String::from_utf8(name).unwrap_or_default()
            } else {
                String::new()
            })
        }
        [_, 0xFF, 0x8F, 0x83, 0xB5, ..] => Some(String::new()),
        _ => None,
    }
}

/// Writes a register read to the short collection and waits for the answer on either: data comes
/// back as a long report, errors as short ones.
fn register<T>(
    short: &HidDevice,
    long: &HidDevice,
    request: [u8; 7],
    reply: impl Fn(&[u8]) -> Option<T>,
) -> Option<T> {
    short.write(&request).ok()?;
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        for hid in [long, short] {
            if let Ok(len) = hid.read_timeout(&mut buf, 10)
                && let Some(r) = reply(&buf[..len])
            {
                return Some(r);
            }
        }
    }
    None
}

pub fn read_slot(short: &HidDevice, long: &HidDevice, n: u8) -> Slot {
    register(short, long, slot_request(n), |f| slot_reply(n, f)).unwrap_or(Slot::Silent)
}

pub fn read_name(short: &HidDevice, long: &HidDevice, n: u8) -> String {
    let request = [0x10, 0xFF, 0x83, 0xB5, 0x40 + n - 1, 0, 0];
    register(short, long, request, |f| name_reply(n, f)).unwrap_or_default()
}

/// The paired slots as (slot, wireless pid, type), or `None` when slot 1 gets no receiver's answer.
/// Stops at the first silent slot, so a receiver costs one exchange per slot it has.
pub fn pairing(mut ask: impl FnMut(u8) -> Slot) -> Option<Vec<(u8, u16, u8)>> {
    let mut slots = Vec::new();
    for n in 1..=SLOTS {
        match ask(n) {
            Slot::Paired(wpid, kind) => slots.push((n, wpid, kind)),
            Slot::Empty => {}
            Slot::Silent if n == 1 => return None,
            Slot::Silent => break,
        }
    }
    Some(slots)
}

/// The devices a long collection reaches. `slots` is its pairing table, `None` when it is no receiver.
pub fn targets(pid: u16, product: &str, slots: Option<Vec<(u8, Paired)>>) -> Vec<Target> {
    let target = |index, paired, known| Target {
        pid,
        index,
        paired,
        known,
        product: product.to_string(),
    };
    let original = matches!(pid, G502_RECEIVER | G515_RECEIVER);
    // The original model on its receiver: the first device of its type that is no other known model.
    let kind = if pid == G515_RECEIVER { PAIRED_KEYBOARD } else { PAIRED_MOUSE };
    let theirs = |p: &Paired| p.kind == kind && !is_g915x(&p.name);
    match slots {
        _ if cable(pid) => vec![target(WIRED_INDEX, None, true)],
        Some(slots) if !slots.is_empty() => {
            let first = slots.iter().position(|(_, p)| theirs(p));
            slots
                .into_iter()
                .enumerate()
                .map(|(i, (n, paired))| target(n, Some(paired), original && first == Some(i)))
                .collect()
        }
        // the original receivers keep their device at slot 1 even when the table cannot be read
        _ if original => vec![target(1, None, true)],
        Some(_) => Vec::new(),
        None => vec![target(WIRED_INDEX, None, false)],
    }
}

/// Whether a device is offered after its first discovery: a known model may be asleep,
/// anything else must answer with a lighting feature.
pub fn offer(known: bool, found: Result<(), String>) -> Result<(), String> {
    match found {
        Err(e) if e == NO_LIGHTING || !known => Err(e),
        _ => Ok(()),
    }
}

/// Zone ids, LED positions and grid of a model without a fixed layout. Per-key boards take the
/// keys they report, any others on an extra row; other devices get one LED per zone.
pub fn discovered_layout(
    kind: DeviceKind,
    reported: &[u8],
    zones: u8,
) -> (Vec<u8>, Vec<[f32; 2]>, [u32; 2]) {
    if reported.is_empty() {
        let n = zones.max(1);
        let (cols, rows) = if kind == DeviceKind::Keyboard {
            (n as u32, 1)
        } else {
            (1, n as u32)
        };
        let leds = Device::grid("", "", kind, [0.0; 4], cols, rows).leds;
        return ((0..n).collect(), leds, [cols, rows]);
    }
    let full = reported.iter().any(|z| (80..=96).contains(z));
    let base = if full { full_layout() } else { tkl_layout() };
    let cols = if full { 22 } else { 18 };
    let mut keys: Vec<(u8, [f32; 2])> = base
        .into_iter()
        .filter(|(zone, _)| reported.contains(zone))
        .collect();
    let extra: Vec<u8> = reported
        .iter()
        .copied()
        .filter(|z| !keys.iter().any(|(k, _)| k == z))
        .collect();
    let rows = if extra.is_empty() { 6 } else { 7 };
    if !extra.is_empty() {
        for (_, at) in &mut keys {
            at[1] *= 6.0 / 7.0;
        }
        let n = extra.len() as f32;
        keys.extend(
            extra
                .iter()
                .enumerate()
                .map(|(i, &z)| (z, [(i as f32 + 0.5) / n, 6.5 / 7.0])),
        );
    }
    let (zones, leds) = keys.into_iter().unzip();
    (zones, leds, [cols, rows])
}

fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        _ => [0.34, 0.12, 0.24, 0.24],
    }
}

impl Lightspeed {
    /// The device a target names. Its features are read now if it is awake, otherwise once it wakes;
    /// only known models are offered while asleep.
    pub fn open(hid: HidDevice, target: Target) -> Result<Self, String> {
        let wired = target.index == WIRED_INDEX;
        let paired_kind = target.paired.as_ref().map(|p| p.kind);
        let (keyboard, model) = if target.known {
            identify(target.pid, target.paired.as_ref())
        } else {
            (paired_kind == Some(PAIRED_KEYBOARD), target.model())
        };
        let link = if wired { " (cable)" } else { " (wireless)" };
        let mut light = Lightspeed {
            hid,
            device: Device::grid("", "", DeviceKind::Other, [0.0; 4], 1, 1),
            index: target.index,
            keys: keyboard || (!target.known && paired_kind != Some(PAIRED_MOUSE)),
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
        let found = light.discover();
        offer(target.known, found).map_err(|e| format!("Logitech {model}: {e}"))?;
        let id = target.id();
        let name = format!("Logitech {model}{link}");
        if !target.known {
            let (kind, zones) = light
                .shape(paired_kind)
                .map_err(|e| format!("Logitech {model}: {e}"))?;
            let (ids, leds, grid) = discovered_layout(kind, &light.reported, zones);
            light.zones = ids;
            light.device = Device {
                id,
                name,
                kind,
                rect: rect(kind),
                grid,
                leds,
            };
            return Ok(light);
        }
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
        let per_key = if self.keys {
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

    /// Kind and zone count of a model without a fixed layout. Older per-key boards are left to
    /// the other HID++ driver.
    fn shape(&self, paired_kind: Option<u8>) -> Result<(DeviceKind, u8), String> {
        let (hid, device) = (&self.hid, self.index);
        if self.per_key.is_none() && self.keys && feature(hid, device, PER_KEY_V1)?.is_some() {
            return Err("per-key lighting of an older kind".into());
        }
        let zones = match self.effects {
            Some((f, Effects::Rgb)) => {
                request(hid, &long(device, f, 0x00, &[0xFF, 0xFF, 0x00]))?[2]
            }
            Some((f, Effects::ColorLed)) => request(hid, &long(device, f, 0x00, &[]))?[0],
            None => 0,
        };
        let kind = match paired_kind {
            _ if self.per_key.is_some() => DeviceKind::Keyboard,
            Some(PAIRED_KEYBOARD) => DeviceKind::Keyboard,
            Some(PAIRED_MOUSE) => DeviceKind::Mouse,
            _ => {
                let kind = feature(hid, device, DEVICE_NAME_TYPE)
                    .ok()
                    .flatten()
                    .and_then(|f| request(hid, &long(device, f, 0x20, &[])).ok());
                match kind.map(|r| r[0]) {
                    Some(0 | 2) => DeviceKind::Keyboard,
                    Some(3..=5) => DeviceKind::Mouse,
                    _ => DeviceKind::Other,
                }
            }
        };
        Ok((kind, zones))
    }

    pub fn id(&self) -> &str {
        &self.device.id
    }

    pub fn wired(&self) -> bool {
        self.index == WIRED_INDEX
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
    fn g915x_and_g515_never_share_an_id() {
        let kb = |name: &str| Paired {
            wpid: 0,
            kind: 1,
            name: name.into(),
        };
        let ids: Vec<(String, bool)> = targets(
            G515_RECEIVER,
            "USB Receiver",
            Some(vec![(1, kb("G915 X LS")), (2, kb("G515 LS TKL"))]),
        )
        .iter()
        .map(|t| (t.id(), t.known))
        .collect();
        assert_eq!(
            ids,
            [
                ("hid:logitech-c356".to_string(), false),
                ("hid:logitech-c547".to_string(), true)
            ]
        );
        let cabled = |pid| targets(pid, "", None)[0].id();
        assert_eq!(cabled(G915X_LS_WIRED), "hid:logitech-c356");
        assert_eq!(cabled(G515_WIRED), "hid:logitech-c547");
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

    fn paired(wpid: u16, kind: u8) -> Paired {
        Paired {
            wpid,
            kind,
            name: String::new(),
        }
    }

    fn ids(pid: u16, slots: Option<Vec<(u8, Paired)>>) -> Vec<(u8, bool, String)> {
        targets(pid, "USB Receiver", slots)
            .iter()
            .map(|t| (t.index, t.known, t.id()))
            .collect()
    }

    #[test]
    fn original_devices_keep_their_ids() {
        let id = |s: &str| s.to_string();
        assert_eq!(
            ids(G502_RECEIVER, None),
            [(1, true, id("hid:logitech-c539"))]
        );
        assert_eq!(
            ids(G502_WIRED, None),
            [(0xFF, true, id("hid:logitech-c539"))]
        );
        assert_eq!(
            ids(G515_WIRED, None),
            [(0xFF, true, id("hid:logitech-c547"))]
        );
        assert_eq!(
            ids(G915X_WIRED, None),
            [(0xFF, true, id("hid:logitech-c359"))]
        );
        assert_eq!(
            ids(G515_RECEIVER, Some(Vec::new())),
            [(1, true, id("hid:logitech-c547"))]
        );
        // whatever slot it sits in, the first device keeps the receiver's id; the next is its own model
        assert_eq!(
            ids(
                G515_RECEIVER,
                Some(vec![(2, paired(0x40A9, 1)), (3, paired(0x4099, 2))])
            ),
            [
                (2, true, id("hid:logitech-c547")),
                (3, false, id("hid:logitech-4099"))
            ]
        );
    }

    #[test]
    fn other_receivers_and_cables() {
        let id = |s: &str| s.to_string();
        let slots = vec![(1, paired(0x40A5, 2)), (4, paired(0, 0))];
        assert_eq!(
            ids(0xC548, Some(slots)),
            [
                (1, false, id("hid:logitech-40a5")),
                (4, false, id("hid:logitech-c548-4"))
            ]
        );
        assert!(ids(0xC548, Some(Vec::new())).is_empty());
        assert_eq!(ids(0xC09D, None), [(0xFF, false, id("hid:logitech-c09d"))]);
        // a cable is never read as a receiver
        assert_eq!(
            ids(G502_WIRED, Some(vec![(1, paired(0x407F, 2))]))[0].0,
            0xFF
        );
    }

    #[test]
    fn unknown_models_are_named_from_pairing_or_product() {
        let mut t = targets(0xC09D, "Logitech G502 X", None).remove(0);
        assert_eq!(t.model(), "G502 X");
        t.product.clear();
        assert_eq!(t.model(), "device c09d");
        let mut slot =
            targets(0xC548, "USB Receiver", Some(vec![(1, paired(0x40A5, 2))])).remove(0);
        assert_eq!(slot.model(), "device 40a5");
        slot.paired.as_mut().unwrap().name = "PRO X 2".into();
        assert_eq!(slot.model(), "PRO X 2");
    }

    #[test]
    fn pairing_reads_only_slots_that_answer() {
        let table = |replies: Vec<Slot>| {
            let mut asked = 0;
            let out = pairing(|n| {
                asked = n;
                replies.get(n as usize - 1).copied().unwrap_or(Slot::Silent)
            });
            (out, asked)
        };
        assert_eq!(
            table(vec![Slot::Paired(0x407F, 2), Slot::Empty, Slot::Silent]),
            (Some(vec![(1, 0x407F, 2)]), 3)
        );
        assert_eq!(table(vec![Slot::Silent]), (None, 1));
        assert_eq!(table(vec![Slot::Empty; 6]), (Some(Vec::new()), 6));
        assert_eq!(
            table(vec![Slot::Empty, Slot::Paired(0x4099, 1)]).0,
            Some(vec![(2, 0x4099, 1)])
        );
    }

    #[test]
    fn slot_replies() {
        assert_eq!(slot_request(2), [0x10, 0xFF, 0x83, 0xB5, 0x21, 0, 0]);
        let mut data = vec![
            0x11, 0xFF, 0x83, 0xB5, 0x21, 0x07, 0x08, 0x40, 0x9F, 0x11, 0x00, 0x02,
        ];
        data.resize(20, 0);
        assert_eq!(slot_reply(2, &data), Some(Slot::Paired(0x409F, 2)));
        assert_eq!(slot_reply(1, &data), None);
        data[7..9].copy_from_slice(&[0, 0]);
        assert_eq!(slot_reply(2, &data), Some(Slot::Empty));
        let error = [0x10, 0xFF, 0x8F, 0x83, 0xB5, 0x03, 0x00];
        assert_eq!(slot_reply(3, &error), Some(Slot::Empty));
        let hidpp2 = [0x11, 0xFF, 0xFF, 0x83, 0xB7, 0x05, 0x00];
        assert_eq!(slot_reply(1, &hidpp2), Some(Slot::Silent));
        assert_eq!(slot_reply(1, &[0x10, 0x01, 0x41, 0x04, 0, 0, 0]), None);
    }

    #[test]
    fn name_replies() {
        let mut frame = vec![0x11, 0xFF, 0x83, 0xB5, 0x41, 5];
        frame.extend(b"G915\0");
        frame.resize(20, 0);
        assert_eq!(name_reply(2, &frame), Some("G915".into()));
        assert_eq!(name_reply(1, &frame), None);
        frame[5] = 15;
        assert_eq!(name_reply(2, &frame), Some(String::new()));
        let error = [0x10, 0xFF, 0x8F, 0x83, 0xB5, 0x03, 0x00];
        assert_eq!(name_reply(2, &error), Some(String::new()));
    }

    #[test]
    fn only_known_models_are_offered_asleep() {
        let asleep = || Err("no reply from the device".to_string());
        assert!(offer(true, asleep()).is_ok());
        assert!(offer(false, asleep()).is_err());
        assert!(offer(false, Err("HID++ error 0x09".into())).is_err());
        assert!(offer(true, Err(NO_LIGHTING.into())).is_err());
        assert!(offer(false, Err(NO_LIGHTING.into())).is_err());
        assert!(offer(false, Ok(())).is_ok());
    }

    #[test]
    fn discovered_layouts() {
        let (zones, leds, grid) = discovered_layout(DeviceKind::Mouse, &[], 3);
        assert_eq!((zones, leds.len(), grid), (vec![0, 1, 2], 3, [1, 3]));
        assert_eq!(discovered_layout(DeviceKind::Other, &[], 0).0, [0]);
        assert_eq!(discovered_layout(DeviceKind::Keyboard, &[], 2).2, [2, 1]);

        let tkl: Vec<u8> = tkl_layout().iter().map(|(z, _)| *z).collect();
        let (zones, _, grid) = discovered_layout(DeviceKind::Keyboard, &tkl[..40], 0);
        assert_eq!((zones.as_slice(), grid), (&tkl[..40], [18, 6]));

        let mut full: Vec<u8> = (1..=111).collect();
        full.extend([153, 210]);
        let (zones, leds, grid) = discovered_layout(DeviceKind::Keyboard, &full, 0);
        assert_eq!(grid, [22, 7]);
        // every reported key gets one LED, the unplaced ones on the extra row
        assert_eq!(zones.len(), full.len());
        assert_eq!(zones[zones.len() - 2..], [153, 210]);
        assert!(leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
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
