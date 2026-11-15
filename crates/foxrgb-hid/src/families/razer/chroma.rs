//! Ported from OpenRGB Controllers/RazerController/RazerController (GPL-2.0-or-later).

use super::{Matrix, Model, model, stacked};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device};
use hidapi::{HidApi, HidDevice};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const REPORT_LEN: usize = 91;
pub const ARGB_LEN: usize = 321;
const ARGS: usize = 9;

const NO_SAVE: u8 = 0x00;
const DEVICE_MODE_SOFTWARE: u8 = 0x03;
const LED_SCROLL_WHEEL: u8 = 0x01;
const LED_LOGO: u8 = 0x04;
const LED_BACKLIGHT: u8 = 0x05;
const LED_ARGB_CH_1: u8 = 0x1A;
const STATUS_NEW: u8 = 0x00;
const STATUS_BUSY: u8 = 0x01;
const STATUS_SUCCESS: u8 = 0x02;
const MAX_PAIRED: usize = 8;
/// LEDs per channel the addressable controller is sized to, its table maximum.
pub const ARGB_CHANNEL_LEDS: u8 = 80;

const ORBWEAVER_CHROMA: u16 = 0x0207;
const TARTARUS_CHROMA: u16 = 0x0208;
const DEATHADDER_CHROMA: u16 = 0x0043;
const DEATHADDER_ESSENTIAL_V2: u16 = 0x0098;
const MAMBA_2012_WIRED: u16 = 0x0024;
const MAMBA_2012_WIRELESS: u16 = 0x0025;
const NAGA_EPIC_CHROMA: u16 = 0x003E;
const LEVIATHAN_V2: u16 = 0x0532;
const LEVIATHAN_V2X: u16 = 0x054A;
const LEVIATHAN_V2_PRO: u16 = 0x0548;
const NOMMO_V2: u16 = 0x055C;
const NOMMO_V2_PRO: u16 = 0x055A;
const CHARGING_PAD_CHROMA: u16 = 0x0F26;
const CHROMA_MUG: u16 = 0x0F07;
const FIREFLY_HYPERFLUX: u16 = 0x0068;
const BLADE_14_2021: u16 = 0x0270;
const BLADE_14_2022: u16 = 0x028C;

pub type Report = [u8; REPORT_LEN];

/// What the controller derives from the PID in its constructor.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub pid: u16,
    pub report_id: u8,
    pub tid: u8,
    pub led_id: u8,
    pub matrix: Matrix,
    pub rows: usize,
    pub cols: usize,
}

impl Params {
    pub fn new(pid: u16) -> Params {
        let m: &Model = model(pid);
        Params {
            pid,
            report_id: if matches!(pid, LEVIATHAN_V2 | LEVIATHAN_V2X | NOMMO_V2 | NOMMO_V2_PRO) {
                0x07
            } else {
                0x00
            },
            tid: m.tid,
            led_id: m.led_id,
            matrix: m.matrix,
            rows: m.rows as usize,
            cols: m.cols as usize,
        }
    }

    /// `GetMaxBrightness`, the brightness Direct starts at.
    pub fn max_brightness(&self) -> u8 {
        if self.pid == DEATHADDER_ESSENTIAL_V2 {
            100
        } else {
            255
        }
    }

    fn keepalive(&self) -> bool {
        matches!(self.pid, BLADE_14_2021 | BLADE_14_2022)
    }
}

/// `razer_create_report`, with `razer_usb_send`'s CRC filled in.
fn report(p: &Params, class: u8, cmd: u8, size: u8, args: &[u8]) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[0] = p.report_id;
    r[2] = p.tid;
    r[6] = size;
    r[7] = class;
    r[8] = cmd;
    let n = args.len().min(80);
    r[ARGS..ARGS + n].copy_from_slice(&args[..n]);
    r[89] = crc(&r);
    r
}

/// `razer_calculate_crc`: XOR of bytes 3 to 88.
pub fn crc(r: &Report) -> u8 {
    r[3..89].iter().fold(0, |a, b| a ^ b)
}

/// `razer_create_response`.
pub fn response(p: &Params) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[0] = p.report_id;
    r[2] = p.tid;
    r
}

/// The check `razer_usb_query` and `razer_usb_send` make on a reply.
pub fn answers(request: &Report, reply: &[u8]) -> bool {
    reply.len() >= ARGS
        && reply[2] == request[2]
        && reply[7] == request[7]
        && reply[8] == request[8]
        && reply[1] != STATUS_NEW
        && reply[1] != STATUS_BUSY
}

pub fn device_mode(p: &Params, mode: u8, param: u8) -> Report {
    report(p, 0x00, 0x04, 0x02, &[mode, param])
}

pub fn get_device_mode(p: &Params) -> Report {
    report(p, 0x00, 0x84, 0x02, &[])
}

pub fn get_wireless_connections(p: &Params) -> Report {
    report(p, 0x00, 0xBF, 0x50, &[])
}

/// `GetPairedDevicePIDs` reading a successful reply.
pub fn paired_pids(reply: &Report) -> Vec<u16> {
    let args = &reply[ARGS..ARGS + 80];
    let slots = args[0] as usize;
    (0..slots)
        .take_while(|s| s * 3 + 3 < 80)
        .map(|s| (args[s * 3 + 2] as u16) << 8 | args[s * 3 + 3] as u16)
        .filter(|&pid| pid != 0x0000 && pid != 0xFFFF)
        .take(MAX_PAIRED)
        .collect()
}

pub fn brightness_standard(p: &Params, led: u8, level: u8) -> Report {
    report(p, 0x03, 0x03, 0x03, &[NO_SAVE, led, level])
}

pub fn brightness_extended(p: &Params, led: u8, level: u8) -> Report {
    report(p, 0x0F, 0x04, 0x03, &[NO_SAVE, led, level])
}

pub fn addressable_sizes(p: &Params, sizes: [u8; 6]) -> Report {
    let mut args = vec![0x06];
    for (i, &s) in sizes.iter().enumerate() {
        args.push(if s == 0 { i as u8 + 1 } else { 0x19 });
        args.push(s);
    }
    report(p, 0x0F, 0x08, 0x0D, &args)
}

pub fn frame_standard(p: &Params, row: u8, start: u8, stop: u8, rgb: &[u8]) -> Report {
    let len = (stop as usize + 1 - start as usize) * 3;
    let mut args = vec![0xFF, row, start, stop];
    args.extend_from_slice(&rgb[..len.min(rgb.len())]);
    report(p, 0x03, 0x0B, (len + 4) as u8, &args)
}

pub fn frame_extended(p: &Params, row: u8, start: u8, stop: u8, rgb: &[u8]) -> Report {
    let len = (stop as usize + 1 - start as usize) * 3;
    let mut args = vec![0x00, 0x00, row, start, stop];
    args.extend_from_slice(&rgb[..len.min(rgb.len())]);
    report(p, 0x0F, 0x03, (len + 5) as u8, &args)
}

pub fn frame_linear(p: &Params, start: u8, stop: u8, rgb: &[u8]) -> Report {
    let len = (stop as usize + 1 - start as usize) * 3;
    let mut args = vec![start, stop];
    args.extend_from_slice(&rgb[..len.min(rgb.len())]);
    report(p, 0x03, 0x0C, 0x32, &args)
}

/// `razer_create_custom_frame_argb_report`.
pub fn frame_argb(row: u8, stop: u8, rgb: &[u8]) -> [u8; ARGB_LEN] {
    let mut r = [0u8; ARGB_LEN];
    r[1] = if row < 5 { 0x04 } else { 0x84 };
    r[2] = row;
    r[3] = row;
    r[5] = stop;
    let n = ((stop as usize + 1) * 3).min(rgb.len()).min(315);
    r[6..6 + n].copy_from_slice(&rgb[..n]);
    r
}

pub fn set_led_rgb(p: &Params, led: u8, rgb: &[u8]) -> Report {
    report(p, 0x03, 0x01, 0x05, &[NO_SAVE, led, rgb[0], rgb[1], rgb[2]])
}

pub fn set_led_effect(p: &Params, led: u8, effect: u8) -> Report {
    report(p, 0x03, 0x02, 0x03, &[NO_SAVE, led, effect.min(5)])
}

pub fn mode_custom_standard(p: &Params) -> Report {
    report(p, 0x03, 0x0A, 0x02, &[0x05, NO_SAVE])
}

pub fn mode_custom_extended(p: &Params) -> Report {
    report(p, 0x0F, 0x02, 0x0C, &[0x00, 0x00, 0x08])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Feature report on the main collection.
    Send(Vec<u8>),
    /// Feature report on the addressable controller's second collection.
    SendArgb(Vec<u8>),
    SleepMs(u64),
}

fn send(r: Report) -> Step {
    Step::Send(r.to_vec())
}

/// `razer_set_brightness`.
pub fn set_brightness(p: &Params, level: u8) -> Vec<Step> {
    match p.matrix {
        Matrix::Standard | Matrix::Linear => vec![send(brightness_standard(p, p.led_id, level))],
        Matrix::Extended => vec![send(brightness_extended(p, p.led_id, level))],
        Matrix::ExtendedArgb => (0..6)
            .map(|ch| send(brightness_extended(p, LED_ARGB_CH_1 + ch, level)))
            .collect(),
        Matrix::Custom if p.pid == ORBWEAVER_CHROMA => {
            vec![send(brightness_standard(p, p.led_id, level))]
        }
        _ => Vec::new(),
    }
}

/// `razer_set_custom_frame`.
pub fn set_custom_frame(p: &Params, row: u8, start: u8, stop: u8, rgb: &[u8]) -> Vec<Step> {
    match p.matrix {
        Matrix::Standard => vec![send(frame_standard(p, row, start, stop, rgb))],
        Matrix::Extended => vec![send(frame_extended(p, row, start, stop, rgb))],
        Matrix::Linear => vec![send(frame_linear(p, start, stop, rgb))],
        Matrix::ExtendedArgb => vec![Step::SendArgb(frame_argb(row, stop, rgb).to_vec())],
        Matrix::Custom => match p.pid {
            TARTARUS_CHROMA => vec![send(set_led_rgb(p, LED_BACKLIGHT, rgb))],
            DEATHADDER_CHROMA => vec![
                send(set_led_rgb(p, LED_SCROLL_WHEEL, rgb)),
                send(set_led_rgb(p, LED_LOGO, &rgb[3..])),
            ],
            NAGA_EPIC_CHROMA => vec![
                send(set_led_rgb(p, LED_SCROLL_WHEEL, rgb)),
                Step::SleepMs(1),
                send(set_led_rgb(p, LED_BACKLIGHT, &rgb[3..])),
            ],
            MAMBA_2012_WIRED | MAMBA_2012_WIRELESS => {
                vec![send(set_led_rgb(p, LED_SCROLL_WHEEL, rgb))]
            }
            ORBWEAVER_CHROMA if row != 3 => {
                vec![send(frame_standard(p, row + 1, start + 1, stop + 1, rgb))]
            }
            ORBWEAVER_CHROMA => {
                let mut adj = [0u8; 18];
                adj[..3].copy_from_slice(&rgb[..3]);
                adj[6..18].copy_from_slice(&rgb[3..15]);
                vec![send(frame_standard(p, row + 1, start + 1, stop + 2, &adj))]
            }
            _ => Vec::new(),
        },
        Matrix::None => Vec::new(),
    }
}

/// `razer_set_mode_custom`.
pub fn set_mode_custom(p: &Params) -> Vec<Step> {
    match p.matrix {
        Matrix::Standard | Matrix::Linear => vec![send(mode_custom_standard(p))],
        Matrix::Extended => vec![send(mode_custom_extended(p))],
        Matrix::Custom => match p.pid {
            TARTARUS_CHROMA => vec![send(set_led_effect(p, LED_BACKLIGHT, 0))],
            DEATHADDER_CHROMA => vec![
                send(set_led_effect(p, LED_SCROLL_WHEEL, 0)),
                send(set_led_effect(p, LED_LOGO, 0)),
            ],
            NAGA_EPIC_CHROMA => vec![
                send(set_led_effect(p, LED_SCROLL_WHEEL, 0)),
                Step::SleepMs(1),
                send(set_led_effect(p, LED_BACKLIGHT, 0)),
            ],
            MAMBA_2012_WIRED | MAMBA_2012_WIRELESS => {
                vec![send(set_led_effect(p, LED_SCROLL_WHEEL, 0))]
            }
            ORBWEAVER_CHROMA => vec![send(mode_custom_standard(p))],
            _ => Vec::new(),
        },
        Matrix::ExtendedArgb | Matrix::None => Vec::new(),
    }
}

/// `SetLEDs`: the device matrix row by row; cells past the last LED stay black.
pub fn set_leds(p: &Params, colors: &[Color]) -> Vec<Step> {
    let mut steps = Vec::new();
    for row in 0..p.rows {
        let mut rgb = vec![0u8; p.cols * 3];
        for col in 0..p.cols {
            if let Some(c) = colors.get(row * p.cols + col) {
                rgb[col * 3..col * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
            }
        }
        steps.push(Step::SleepMs(1));
        steps.extend(set_custom_frame(p, row as u8, 0, p.cols as u8 - 1, &rgb));
    }
    steps.push(Step::SleepMs(1));
    steps.extend(set_mode_custom(p));
    steps
}

/// The addressable controller packs each zone at 80 LEDs per row (`RGBController_RazerAddressable::DeviceUpdateLEDs`).
pub fn argb_buffer(colors: &[Color]) -> Vec<Color> {
    let per = ARGB_CHANNEL_LEDS as usize;
    let mut buf = vec![Color::BLACK; per * 6];
    for (i, c) in colors.iter().take(per * 6).enumerate() {
        buf[i] = *c;
    }
    buf
}

/// What one push sends. The first push takes control the way Direct mode does in OpenRGB.
pub fn plan(p: &Params, first: bool, colors: &[Color]) -> Vec<Step> {
    let mut steps = Vec::new();
    if p.matrix == Matrix::ExtendedArgb {
        // DeviceConfigureZone, then DeviceUpdateMode(Direct): frame first, brightness after.
        if first {
            steps.push(send(addressable_sizes(p, [ARGB_CHANNEL_LEDS; 6])));
        }
        steps.extend(set_leds(p, &argb_buffer(colors)));
        if first {
            steps.extend(set_brightness(p, 255));
        }
        return steps;
    }
    if first {
        if matches!(
            p.pid,
            CHARGING_PAD_CHROMA | CHROMA_MUG | FIREFLY_HYPERFLUX | LEVIATHAN_V2_PRO
        ) {
            steps.push(send(device_mode(p, DEVICE_MODE_SOFTWARE, 0x00)));
        }
        steps.extend(set_brightness(p, p.max_brightness()));
    }
    steps.extend(set_leds(p, colors));
    steps
}

struct Unit {
    p: Params,
    device: Device,
    wait: bool,
    sent: Option<Vec<Color>>,
}

pub struct Chroma {
    hid: HidDevice,
    argb: Option<HidDevice>,
    units: Vec<Unit>,
    last_keepalive: Instant,
}

fn device_for(id: String, name: String, pid: u16) -> Device {
    let m = model(pid);
    if m.matrix == Matrix::ExtendedArgb {
        let lines = [(1, ARGB_CHANNEL_LEDS); 6];
        return stacked(id, name, m.kind, &lines);
    }
    stacked(id, name, m.kind, m.zones)
}

/// `DetectRazerControllers`, including devices paired to a wireless receiver.
pub fn open(api: &HidApi, info: &Info, detector: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let id = info.device_id("razer");
    let mut units = vec![Unit {
        p: Params::new(info.pid),
        device: device_for(id.clone(), detector.name.into(), info.pid),
        wait: false,
        sent: None,
    }];
    let receiver = model(info.pid).name;
    for pid in query_paired(&hid, &units[0].p) {
        if pid == info.pid || !super::listed(pid) {
            continue;
        }
        let name = format!("{} ({receiver})", model(pid).name);
        units.push(Unit {
            p: Params::new(pid),
            device: device_for(format!("{id}-{pid:04x}"), name, pid),
            wait: true,
            sent: None,
        });
    }
    Ok(Box::new(Chroma {
        hid,
        argb: None,
        units,
        last_keepalive: Instant::now(),
    }))
}

/// `DetectRazerARGBControllers`: interface 0 for commands, interface 1 for frames.
pub fn open_argb(
    api: &HidApi,
    info: &Info,
    detector: &Detector,
) -> Result<Box<dyn Backend>, String> {
    let same = |d: &&hidapi::DeviceInfo| d.vendor_id() == info.vid && d.product_id() == info.pid;
    let path_of = |d: &hidapi::DeviceInfo| d.path().to_string_lossy().into_owned();
    let firsts: Vec<String> = api
        .device_list()
        .filter(same)
        .filter(|d| d.interface_number() == 0)
        .map(path_of)
        .collect();
    let seconds: Vec<&hidapi::DeviceInfo> = api
        .device_list()
        .filter(same)
        .filter(|d| d.interface_number() == 1)
        .collect();
    let index = firsts
        .iter()
        .position(|p| *p == info.path)
        .ok_or("interface 0 not listed")?;
    let second = seconds.get(index).ok_or("interface 1 not found")?;
    let argb = second.open_device(api).map_err(|e| e.to_string())?;
    let hid = info.open(api)?;
    let id = info.device_id("razer");
    Ok(Box::new(Chroma {
        hid,
        argb: Some(argb),
        units: vec![Unit {
            p: Params::new(info.pid),
            device: device_for(id, detector.name.into(), info.pid),
            wait: false,
            sent: None,
        }],
        last_keepalive: Instant::now(),
    }))
}

/// `razer_usb_query` for the wireless connection list; no reply means no paired devices.
fn query_paired(hid: &HidDevice, p: &Params) -> Vec<u16> {
    let request = get_wireless_connections(p);
    for _ in 0..3 {
        sleep(Duration::from_millis(2));
        if hid.send_feature_report(&request).is_err() {
            return Vec::new();
        }
        for _ in 0..25 {
            let mut reply = response(p);
            sleep(Duration::from_millis(2));
            if hid.get_feature_report(&mut reply).is_err() {
                continue;
            }
            if answers(&request, &reply) {
                return if reply[1] == STATUS_SUCCESS {
                    paired_pids(&reply)
                } else {
                    Vec::new()
                };
            }
        }
    }
    Vec::new()
}

impl Chroma {
    fn run(&self, unit: usize, steps: &[Step]) -> Result<(), String> {
        let u = &self.units[unit];
        for step in steps {
            match step {
                Step::SleepMs(ms) => sleep(Duration::from_millis(*ms)),
                Step::SendArgb(bytes) => {
                    let argb = self.argb.as_ref().ok_or("no addressable interface")?;
                    argb.send_feature_report(bytes).map_err(|e| e.to_string())?;
                }
                Step::Send(bytes) => {
                    self.hid
                        .send_feature_report(bytes)
                        .map_err(|e| e.to_string())?;
                    if u.wait {
                        self.await_reply(&u.p, bytes);
                    }
                }
            }
        }
        Ok(())
    }

    /// `razer_usb_send` with `wait_for_response`, used for devices behind a receiver.
    fn await_reply(&self, p: &Params, sent: &[u8]) {
        let mut request = [0u8; REPORT_LEN];
        request.copy_from_slice(&sent[..REPORT_LEN]);
        for _ in 0..50 {
            let mut reply = response(p);
            sleep(Duration::from_millis(2));
            if self.hid.get_feature_report(&mut reply).is_err() || answers(&request, &reply) {
                break;
            }
        }
    }

    /// `KeepaliveThreadFunction`: its timestamp is never set, so it asks for the mode every second.
    fn keepalive(&mut self) {
        if self.last_keepalive.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_keepalive = Instant::now();
        for u in self.units.iter().filter(|u| u.p.keepalive()) {
            sleep(Duration::from_millis(2));
            let _ = self.hid.send_feature_report(&get_device_mode(&u.p));
            sleep(Duration::from_millis(5));
            let mut reply = response(&u.p);
            let _ = self.hid.get_feature_report(&mut reply);
        }
    }
}

impl Backend for Chroma {
    fn name(&self) -> &str {
        "razer chroma"
    }

    fn devices(&self) -> Vec<Device> {
        self.units.iter().map(|u| u.device.clone()).collect()
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        self.keepalive();
        let unit = self
            .units
            .iter()
            .position(|u| u.device.id == device_id)
            .ok_or_else(|| format!("unknown device {device_id}"))?;
        if self.units[unit].sent.as_deref() == Some(colors) {
            return Ok(());
        }
        let first = self.units[unit].sent.is_none();
        let steps = plan(&self.units[unit].p, first, colors);
        match self.run(unit, &steps) {
            Ok(()) => {
                self.units[unit].sent = Some(colors.to_vec());
                Ok(())
            }
            Err(e) => {
                self.units[unit].sent = None;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sends(steps: &[Step]) -> Vec<Vec<u8>> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Send(b) | Step::SendArgb(b) => Some(b.clone()),
                Step::SleepMs(_) => None,
            })
            .collect()
    }

    fn head(r: &[u8]) -> [u8; 9] {
        r[..9].try_into().unwrap()
    }

    // razer_create_report and razer_calculate_crc.
    #[test]
    fn report_header_and_crc() {
        let p = Params::new(0x0241);
        let r = mode_custom_extended(&p);
        assert_eq!(r.len(), 91);
        assert_eq!(
            head(&r),
            [0x00, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x0C, 0x0F, 0x02]
        );
        assert_eq!(r[9..12], [0x00, 0x00, 0x08]);
        assert_eq!(r[89], 0x0C ^ 0x0F ^ 0x02 ^ 0x08);
        assert_eq!(r[90], 0);
    }

    // RazerController constructor: Leviathan V2 uses report id 7.
    #[test]
    fn leviathan_uses_report_seven() {
        let p = Params::new(LEVIATHAN_V2);
        assert_eq!(p.report_id, 0x07);
        assert_eq!(mode_custom_extended(&p)[0], 0x07);
        assert_eq!(response(&p)[0], 0x07);
        assert_eq!(Params::new(0x0241).report_id, 0);
    }

    // razer_create_brightness_extended_matrix_report / razer_create_brightness_standard_report.
    #[test]
    fn brightness_reports() {
        let p = Params::new(0x0241);
        let r = brightness_extended(&p, 0x05, 0xFF);
        assert_eq!(head(&r), [0, 0, 0x1F, 0, 0, 0, 0x03, 0x0F, 0x04]);
        assert_eq!(r[9..12], [0x00, 0x05, 0xFF]);
        assert_eq!(r[89], 0x03 ^ 0x0F ^ 0x04 ^ 0x05 ^ 0xFF);
        let s = Params::new(0x0203);
        let r = brightness_standard(&s, 0x05, 0x80);
        assert_eq!(head(&r), [0, 0, 0x3F, 0, 0, 0, 0x03, 0x03, 0x03]);
        assert_eq!(r[9..12], [0x00, 0x05, 0x80]);
        assert_eq!(
            r[89],
            0x03 ^ 0x05 ^ 0x80,
            "size, class and cmd are all 0x03"
        );
    }

    // razer_create_custom_frame_extended_matrix_report.
    #[test]
    fn extended_frame_report() {
        let p = Params::new(0x0241);
        let rgb: Vec<u8> = (1..=66).collect();
        let r = frame_extended(&p, 2, 0, 21, &rgb);
        assert_eq!(head(&r), [0, 0, 0x1F, 0, 0, 0, 66 + 5, 0x0F, 0x03]);
        assert_eq!(r[9..14], [0x00, 0x00, 2, 0, 21]);
        assert_eq!(r[14..80], rgb[..]);
        assert!(r[80..89].iter().all(|&b| b == 0));
        let x = rgb.iter().fold(71 ^ 0x0F ^ 0x03 ^ 2 ^ 21, |a, b| a ^ b);
        assert_eq!(r[89], x);
    }

    // razer_create_custom_frame_standard_matrix_report.
    #[test]
    fn standard_frame_report() {
        let p = Params::new(0x0203);
        let rgb: Vec<u8> = (10..76).collect();
        let r = frame_standard(&p, 5, 0, 21, &rgb);
        assert_eq!(head(&r), [0, 0, 0x3F, 0, 0, 0, 66 + 4, 0x03, 0x0B]);
        assert_eq!(r[9..13], [0xFF, 5, 0, 21]);
        assert_eq!(r[13..79], rgb[..]);
        assert_eq!(r[79], 0);
    }

    // razer_create_custom_frame_linear_report: fixed size 0x32, no row.
    #[test]
    fn linear_frame_report() {
        let p = Params {
            matrix: Matrix::Linear,
            ..Params::new(0x0C00)
        };
        let r = frame_linear(&p, 0, 14, &[7; 45]);
        assert_eq!(r[6..9], [0x32, 0x03, 0x0C]);
        assert_eq!(r[9..11], [0, 14]);
        assert!(r[11..56].iter().all(|&b| b == 7));
        assert_eq!(r[56], 0);
    }

    // razer_create_custom_frame_argb_report.
    #[test]
    fn argb_frame_report() {
        let rgb: Vec<u8> = (0..240).map(|i| i as u8).collect();
        let r = frame_argb(2, 79, &rgb);
        assert_eq!(r.len(), 321);
        assert_eq!(r[..6], [0x00, 0x04, 2, 2, 0, 79]);
        assert_eq!(r[6..246], rgb[..]);
        assert!(r[246..].iter().all(|&b| b == 0));
        assert_eq!(frame_argb(5, 79, &rgb)[1], 0x84);
        assert_eq!(frame_argb(4, 79, &rgb)[1], 0x04);
    }

    // razer_create_addressable_size_report.
    #[test]
    fn addressable_size_report() {
        let p = Params::new(0x0F1F);
        let r = addressable_sizes(&p, [80, 0, 80, 80, 80, 80]);
        assert_eq!(head(&r), [0, 0, 0x3F, 0, 0, 0, 0x0D, 0x0F, 0x08]);
        assert_eq!(
            r[9..22],
            [
                0x06, 0x19, 80, 0x02, 0, 0x19, 80, 0x19, 80, 0x19, 80, 0x19, 80
            ]
        );
    }

    // razer_create_set_led_rgb_report / razer_create_set_led_effect_report.
    #[test]
    fn led_rgb_and_effect_reports() {
        let p = Params::new(DEATHADDER_CHROMA);
        let r = set_led_rgb(&p, LED_LOGO, &[1, 2, 3]);
        assert_eq!(r[6..9], [0x05, 0x03, 0x01]);
        assert_eq!(r[9..14], [0x00, 0x04, 1, 2, 3]);
        let e = set_led_effect(&p, LED_SCROLL_WHEEL, 9);
        assert_eq!(e[6..9], [0x03, 0x03, 0x02]);
        assert_eq!(e[9..12], [0x00, 0x01, 0x05]);
        assert_eq!(set_led_effect(&p, 1, 0)[11], 0);
    }

    // razer_create_device_mode_report, razer_get_device_mode, GetPairedDevicePIDs.
    #[test]
    fn mode_and_query_reports() {
        let p = Params::new(CHROMA_MUG);
        let r = device_mode(&p, 0x03, 0x00);
        assert_eq!(r[6..11], [0x02, 0x00, 0x04, 0x03, 0x00]);
        assert_eq!(r[89], 0x02 ^ 0x04 ^ 0x03);
        let g = get_device_mode(&p);
        assert_eq!(g[6..9], [0x02, 0x00, 0x84]);
        let w = get_wireless_connections(&p);
        assert_eq!(w[6..9], [0x50, 0x00, 0xBF]);
        assert_eq!(w[89], 0x50 ^ 0xBF);
    }

    // GetPairedDevicePIDs.
    #[test]
    fn paired_pids_from_reply() {
        let mut reply = [0u8; REPORT_LEN];
        reply[1] = STATUS_SUCCESS;
        reply[9] = 3;
        reply[9 + 2] = 0x00;
        reply[9 + 3] = 0x7C;
        reply[9 + 5] = 0xFF;
        reply[9 + 6] = 0xFF;
        reply[9 + 8] = 0x00;
        reply[9 + 9] = 0x8E;
        assert_eq!(paired_pids(&reply), vec![0x007C, 0x008E]);
        reply[9] = 0;
        assert!(paired_pids(&reply).is_empty());
    }

    // razer_usb_query's reply check.
    #[test]
    fn reply_matching() {
        let p = Params::new(0x0241);
        let req = get_wireless_connections(&p);
        let mut reply = response(&p);
        reply[7] = 0x00;
        reply[8] = 0xBF;
        assert!(!answers(&req, &reply), "status new");
        reply[1] = STATUS_BUSY;
        assert!(!answers(&req, &reply));
        reply[1] = STATUS_SUCCESS;
        assert!(answers(&req, &reply));
        reply[8] = 0x84;
        assert!(!answers(&req, &reply));
    }

    // RGBController_Razer::DeviceUpdateMode(Direct) then DeviceUpdateLEDs -> SetLEDs.
    #[test]
    fn first_extended_push_sets_brightness_then_rows_then_custom_mode() {
        let p = Params::new(0x0241);
        let colors = vec![Color(1, 2, 3); 132];
        let steps = plan(&p, true, &colors);
        let s = sends(&steps);
        assert_eq!(s.len(), 1 + 6 + 1);
        assert_eq!(s[0][..], brightness_extended(&p, 0x05, 255)[..]);
        for row in 0..6 {
            assert_eq!(s[1 + row][7..9], [0x0F, 0x03]);
            assert_eq!(s[1 + row][11], row as u8);
            assert_eq!(s[1 + row][14..17], [1, 2, 3]);
        }
        assert_eq!(s[7][..], mode_custom_extended(&p)[..]);
        assert_eq!(steps.iter().filter(|s| **s == Step::SleepMs(1)).count(), 7);
        assert_eq!(sends(&plan(&p, false, &colors)).len(), 7);
    }

    // SetLEDs on the BlackWidow V4: 156 LEDs in an 8x23 matrix.
    #[test]
    fn short_led_list_pads_matrix_with_black() {
        let p = Params::new(0x0287);
        let mut colors = vec![Color::BLACK; 156];
        for (i, c) in colors.iter_mut().enumerate().skip(138) {
            *c = Color(i as u8, 0, 0);
        }
        let s = sends(&plan(&p, false, &colors));
        assert_eq!(s.len(), 9);
        let row6 = &s[6];
        assert_eq!(row6[6], 23 * 3 + 5);
        assert_eq!(row6[11..14], [6, 0, 22]);
        for col in 0..23 {
            let expect = if col < 18 { 138 + col as u8 } else { 0 };
            assert_eq!(row6[14 + col * 3], expect, "col {col}");
        }
        assert!(s[7][14..83].iter().all(|&b| b == 0));
    }

    // SetLEDs with RAZER_MATRIX_TYPE_STANDARD and razer_set_mode_custom.
    #[test]
    fn standard_push_ends_with_custom_mode() {
        let p = Params::new(0x0203);
        let s = sends(&plan(&p, true, &[Color(9, 9, 9); 132]));
        assert_eq!(s[0][..], brightness_standard(&p, 0x05, 255)[..]);
        assert_eq!(s[1][7..13], [0x03, 0x0B, 0xFF, 0, 0, 21]);
        let last = s.last().unwrap();
        assert_eq!(last[6..11], [0x02, 0x03, 0x0A, 0x05, 0x00]);
    }

    // GetMaxBrightness.
    #[test]
    fn deathadder_essential_v2_direct_brightness_is_100() {
        let p = Params::new(DEATHADDER_ESSENTIAL_V2);
        let s = sends(&plan(&p, true, &[Color::BLACK; 2]));
        assert_eq!(s[0][11], 100);
    }

    // RazerController constructor: the mug is switched to software mode.
    #[test]
    fn mug_gets_software_mode_first() {
        let p = Params::new(CHROMA_MUG);
        let s = sends(&plan(&p, true, &[Color::BLACK; 15]));
        assert_eq!(s[0][..], device_mode(&p, 0x03, 0x00)[..]);
        assert_eq!(p.matrix, Matrix::Linear);
        assert_eq!(s[1][..], brightness_standard(&p, p.led_id, 255)[..]);
    }

    // razer_set_custom_frame / razer_set_mode_custom, DeathAdder Chroma case.
    #[test]
    fn deathadder_chroma_custom_frame() {
        let p = Params::new(DEATHADDER_CHROMA);
        assert_eq!(p.matrix, Matrix::Custom);
        let s = sends(&plan(&p, true, &[Color(1, 2, 3), Color(4, 5, 6)]));
        assert_eq!(s.len(), 4);
        assert_eq!(s[0][..], set_led_rgb(&p, LED_SCROLL_WHEEL, &[1, 2, 3])[..]);
        assert_eq!(s[1][..], set_led_rgb(&p, LED_LOGO, &[4, 5, 6])[..]);
        assert_eq!(s[2][..], set_led_effect(&p, LED_SCROLL_WHEEL, 0)[..]);
        assert_eq!(s[3][..], set_led_effect(&p, LED_LOGO, 0)[..]);
    }

    // razer_set_custom_frame, Naga Epic Chroma case.
    #[test]
    fn naga_epic_sleeps_between_leds() {
        let p = Params::new(NAGA_EPIC_CHROMA);
        let steps = set_custom_frame(&p, 0, 0, 1, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1], Step::SleepMs(1));
        assert_eq!(sends(&steps)[1][10..14], [LED_BACKLIGHT, 4, 5, 6]);
    }

    // razer_set_custom_frame, Mamba 2012 and Tartarus Chroma cases.
    #[test]
    fn single_led_custom_devices() {
        let p = Params::new(MAMBA_2012_WIRED);
        let s = sends(&plan(&p, true, &[Color(7, 8, 9)]));
        assert_eq!(s.len(), 2);
        assert_eq!(s[0][9..14], [0, LED_SCROLL_WHEEL, 7, 8, 9]);
        let t = Params::new(TARTARUS_CHROMA);
        let s = sends(&plan(&t, true, &[Color(7, 8, 9)]));
        assert_eq!(s[0][9..14], [0, LED_BACKLIGHT, 7, 8, 9]);
        assert_eq!(s[1][9..12], [0, LED_BACKLIGHT, 0]);
    }

    // razer_set_custom_frame / razer_set_brightness, Orbweaver Chroma case.
    #[test]
    fn orbweaver_shifts_rows_and_splits_the_last() {
        let p = Params::new(ORBWEAVER_CHROMA);
        let colors: Vec<Color> = (0..20).map(|i| Color(i, i, i)).collect();
        let s = sends(&plan(&p, true, &colors));
        assert_eq!(s.len(), 1 + 4 + 1);
        assert_eq!(s[0][..], brightness_standard(&p, 0x05, 255)[..]);
        assert_eq!(s[1][6], 15 + 4);
        assert_eq!(s[1][9..13], [0xFF, 1, 1, 5]);
        assert_eq!(s[1][13..16], [0, 0, 0]);
        let last = &s[4];
        assert_eq!(last[6], 18 + 4);
        assert_eq!(last[9..13], [0xFF, 4, 1, 6]);
        assert_eq!(last[13..16], [15, 15, 15]);
        assert_eq!(last[16..19], [0, 0, 0]);
        assert_eq!(last[19..22], [16, 16, 16]);
        assert_eq!(last[28..31], [19, 19, 19]);
        assert_eq!(s[5][..], mode_custom_standard(&p)[..]);
    }

    // RGBController_RazerAddressable::DeviceConfigureZone, DeviceUpdateMode(Direct).
    #[test]
    fn argb_first_push() {
        let p = Params::new(0x0F1F);
        assert_eq!(p.matrix, Matrix::ExtendedArgb);
        let mut colors = vec![Color::BLACK; 480];
        colors[80] = Color(1, 2, 3);
        colors[479] = Color(4, 5, 6);
        let steps = plan(&p, true, &colors);
        let s = sends(&steps);
        assert_eq!(s.len(), 1 + 6 + 6);
        assert_eq!(s[0][..], addressable_sizes(&p, [80; 6])[..]);
        assert!(matches!(steps[2], Step::SendArgb(_)));
        assert_eq!(s[2][..9], [0, 0x04, 1, 1, 0, 79, 1, 2, 3]);
        assert_eq!(s[6][1], 0x84);
        assert_eq!(s[6][6 + 79 * 3..6 + 80 * 3], [4, 5, 6]);
        for (i, ch) in (0x1A..=0x1F).enumerate() {
            assert_eq!(s[7 + i][..], brightness_extended(&p, ch, 255)[..]);
        }
        let later = sends(&plan(&p, false, &colors));
        assert_eq!(later.len(), 6);
    }

    #[test]
    fn argb_device_has_six_lines_of_80() {
        let d = device_for("x".into(), "X".into(), 0x0F1F);
        assert_eq!(d.leds.len(), 480);
        assert_eq!(d.grid, [80, 6]);
    }

    #[test]
    fn devices_match_openrgb_led_counts() {
        assert_eq!(device_for("x".into(), "X".into(), 0x0241).leds.len(), 132);
        assert_eq!(device_for("x".into(), "X".into(), 0x0287).leds.len(), 156);
        assert_eq!(
            device_for("x".into(), "X".into(), MAMBA_2012_WIRED)
                .leds
                .len(),
            1
        );
    }

    // SignalRGB Razer_Nommo_Speakers.js: Nommo V2 pair uses report id 7 and one 12-LED row.
    #[test]
    fn nommo_v2_uses_report_seven() {
        for pid in [NOMMO_V2, NOMMO_V2_PRO] {
            let p = Params::new(pid);
            assert_eq!(p.report_id, 0x07);
            let s = sends(&plan(&p, false, &[Color(1, 2, 3); 12]));
            assert_eq!(s[0][..9], [0x07, 0, 0x1F, 0, 0, 0, 12 * 3 + 5, 0x0F, 0x03]);
            assert_eq!(s[0][9..14], [0, 0, 0, 0, 11]);
        }
    }

    // SignalRGB Razer_Soundbar.js: Leviathan V2 Pro goes to software mode, then two rows of 15.
    #[test]
    fn leviathan_v2_pro_software_mode_then_two_rows() {
        let p = Params::new(LEVIATHAN_V2_PRO);
        assert_eq!(p.report_id, 0x00);
        let s = sends(&plan(&p, true, &[Color::BLACK; 30]));
        assert_eq!(s[0][..], device_mode(&p, 0x03, 0x00)[..]);
        assert_eq!(s[2][6], 15 * 3 + 5);
        assert_eq!(s[2][9..14], [0, 0, 0, 0, 14]);
        assert_eq!(s[3][11], 1);
    }

    // SignalRGB Razer_Modern_Mouse.js / Razer_Modern_Keyboard.js: rows sized from the plugin's LED tables.
    #[test]
    fn signalrgb_devices_led_counts() {
        let n = |pid| device_for("x".into(), "X".into(), pid).leds.len();
        assert_eq!(n(0x00C7), 18);
        assert_eq!(n(0x0096), 2);
        assert_eq!(n(0x02CC), 154);
        assert_eq!(n(0x023E), 126);
        assert_eq!(n(0x02B0), 75);
        assert_eq!(n(0x055A), 12);
    }
}
