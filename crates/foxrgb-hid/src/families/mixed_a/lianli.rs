//! Ported from OpenRGB Controllers/LianLiController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::common::{Driver, FANS, Hid, Io, line, rows};
use crate::registry::{Detector, Info};

const ENE: u16 = 0x0CF2;
const NUVOTON: u16 = 0x0416;
const REPORT: u8 = 0xE0;

/// The firmware tag after the last '-' of the product string, as ReadVersion / GetFirmwareVersionString cut it.
pub fn firmware(product: &str) -> &str {
    let tail = product.rsplit('-').next().unwrap_or("");
    tail.trim_end()
}

fn buf(len: usize, head: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[..head.len()].copy_from_slice(head);
    b
}

/// Each channel at the user's length.
fn sized(id: &str, channels: usize, max: usize) -> Vec<usize> {
    (0..channels)
        .map(|i| crate::lengths::length(id, i, &format!("Channel {}", i + 1), max, 0, max))
        .collect()
}

/// One row per channel, each at its own length.
fn hub(id: String, name: &str, max: usize, counts: &[usize]) -> Device {
    let mut d = rows(id, name, DeviceKind::Other, FANS, max, counts.len());
    let n = counts.len() as f32;
    d.leds = counts
        .iter()
        .enumerate()
        .flat_map(|(r, &c)| {
            (0..c).map(move |i| [(i as f32 + 0.5) / c as f32, (r as f32 + 0.5) / n])
        })
        .collect();
    d
}

/// Uni Hub SL: one LED per fan, four fans on each of four channels.
pub const SL_FANS: usize = 4;

/// LianLiUniHubSLController::SendActivate.
pub fn sl_activate(ch: u8, fans: u8) -> Vec<u8> {
    buf(11, &[REPORT, 0x10, 0x32, 0x10 * ch + fans])
}

/// SendMerge, not merged.
pub fn sl_merge() -> Vec<u8> {
    buf(11, &[REPORT, 0x10, 0x34])
}

/// SendColor with FillStaticColorBuffer (each fan's colour on its 16 LEDs, R B G order).
pub fn sl_color(ch: u8, fans: &[Color]) -> Vec<u8> {
    let mut b = buf(2 + 192, &[REPORT, 0x30 + ch]);
    for (f, c) in fans.iter().take(SL_FANS).enumerate() {
        for led in 0..16 {
            let i = 2 + (f * 16 + led) * 3;
            b[i..i + 3].copy_from_slice(&[c.0, c.2, c.1]);
        }
    }
    b
}

/// SendMode for Static: speed 0 -> 0x02, direction left -> 0x00, brightness 4 -> 0x00.
pub fn sl_mode(ch: u8) -> Vec<u8> {
    buf(11, &[REPORT, 0x10 + ch, 0x01, 0x02, 0x00, 0x00])
}

struct UniHubSl {
    counts: Vec<usize>,
}

impl Driver for UniHubSl {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let send = |io: &mut dyn Io, p: Vec<u8>| -> Result<(), String> {
            io.write(&p)?;
            io.sleep(5);
            Ok(())
        };
        for (ch, &n) in self.counts.iter().enumerate() {
            send(io, sl_activate(ch as u8, n as u8))?;
        }
        send(io, sl_merge())?;
        let mut at = 0;
        for (ch, &n) in self.counts.iter().enumerate() {
            send(io, sl_color(ch as u8, &colors[at..at + n]))?;
            send(io, sl_mode(ch as u8))?;
            at += n;
        }
        Ok(())
    }
}

fn open_sl(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    if firmware(&info.product) != "v1.8" {
        return Err(format!("{} firmware is not v1.8", d.name));
    }
    let id = info.device_id("lianli");
    let counts = sized(&id, 4, SL_FANS);
    let dev = hub(id, d.name, SL_FANS, &counts);
    Ok(Hid::boxed(
        "lian li uni hub sl",
        info.port(api)?,
        UniHubSl { counts },
        dev,
    ))
}

/// Uni Hub AL: four fans per channel, each 8 fan LEDs then 12 edge LEDs.
pub const AL_LEDS: usize = 80;

/// UNIHUB_AL_LED_LIMITER: bright whites are held at 153.
fn al_limit(c: Color) -> Color {
    if c.0 > 153 && c.0 == c.1 && c.0 == c.2 {
        Color(153, 153, 153)
    } else {
        c
    }
}

/// SetChannelLEDs: start, fan data, commit, then the same for the edge LEDs.
pub fn al_channel(ch: u8, colors: &[Color], brightness: f32) -> Vec<Vec<u8>> {
    if colors.is_empty() {
        return Vec::new();
    }
    let mut fan = [0u8; 96];
    let mut edge = [0u8; 144];
    let scale = |v: u8| (v as f32 * brightness) as u8;
    for (i, c) in colors.iter().enumerate() {
        let c = al_limit(*c);
        let (f, m) = (i / 20, i % 20);
        let px = [scale(c.0), scale(c.2), scale(c.1)];
        if m < 8 {
            let at = (m + f * 8) * 3;
            fan[at..at + 3].copy_from_slice(&px);
        } else {
            let at = (m - 8 + f * 12) * 3;
            edge[at..at + 3].copy_from_slice(&px);
        }
    }
    let fans = (colors.len() - 1) / 20 + 1;
    let mut out = Vec::new();
    for (side, data, per) in [(0u8, &fan[..], 8), (1, &edge[..], 12)] {
        out.push(buf(65, &[REPORT, 0x10, 0x40, ch + 1, fans as u8]));
        let mut p = buf(146, &[REPORT, 0x30 + side + ch * 2]);
        p[2..2 + fans * per * 3].copy_from_slice(&data[..fans * per * 3]);
        out.push(p);
        out.push(buf(
            65,
            &[REPORT, 0x10 + side + ch * 2, 0x01, 0x02, 0x00, 0x00],
        ));
    }
    out
}

struct UniHubAl {
    counts: Vec<usize>,
}

impl Driver for UniHubAl {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for (ch, &n) in self.counts.iter().enumerate() {
            for p in al_channel(ch as u8, &colors[at..at + n], 1.0) {
                io.write(&p)?;
                io.sleep(5);
            }
            at += n;
        }
        Ok(())
    }
}

fn open_al(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    match firmware(&info.product).get(..4) {
        Some("v1.7") => {}
        Some("v1.0") => return Err("Uni Hub AL v1.0 is driven over libusb".into()),
        _ => return Err(format!("{} firmware is not v1.7", d.name)),
    }
    let id = info.device_id("lianli");
    let counts = sized(&id, 4, AL_LEDS);
    let dev = hub(id, d.name, AL_LEDS, &counts);
    Ok(Hid::boxed(
        "lian li uni hub al",
        info.port(api)?,
        UniHubAl { counts },
        dev,
    ))
}

/// Sends one output report as a HID write would, for a hub bound to WinUSB.
pub trait Reports: Send {
    fn output(&mut self, report: &[u8]) -> Result<(), String>;
}

struct OverUsb(Box<dyn Reports>);

impl Io for OverUsb {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.0.output(data)
    }
    fn send_feature(&mut self, _: &[u8]) -> Result<(), String> {
        Err("no feature reports over WinUSB".into())
    }
    fn get_feature(&mut self, _: &mut [u8]) -> Result<usize, String> {
        Err("no feature reports over WinUSB".into())
    }
    fn read(&mut self, _: &mut [u8], _: i32) -> Result<usize, String> {
        Ok(0)
    }
    fn sleep(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Uni Hub SL v1.8 or AL v1.7 lighting over `out`; `None` for firmware without the HID protocol.
pub fn over_usb(
    pid: u16,
    product: &str,
    id: String,
    name: &str,
    out: Box<dyn Reports>,
) -> Option<Box<dyn Backend>> {
    let io = OverUsb(out);
    match (pid, firmware(product)) {
        (0xA100, "v1.8") => {
            let counts = sized(&id, 4, SL_FANS);
            let dev = hub(id, name, SL_FANS, &counts);
            Some(Hid::boxed(
                "lian li uni hub sl",
                io,
                UniHubSl { counts },
                dev,
            ))
        }
        (0xA101, f) if f.starts_with("v1.7") => {
            let counts = sized(&id, 4, AL_LEDS);
            let dev = hub(id, name, AL_LEDS, &counts);
            Some(Hid::boxed(
                "lian li uni hub al",
                io,
                UniHubAl { counts },
                dev,
            ))
        }
        _ => None,
    }
}

/// Uni Hub SL V2 and SL Infinity: six fans of 16 LEDs per channel.
pub const V2_LEDS: usize = 96;

/// brightnessLimit: the R+G+B sum is held at 460.
fn v2_limit(c: Color) -> f32 {
    let sum = c.0 as u32 + c.1 as u32 + c.2 as u32;
    if sum > 460 { 460.0 / sum as f32 } else { 1.0 }
}

/// SetChannelLEDs of the SL V2 (`infinity` false) and SL Infinity: start, colours, commit.
pub fn v2_channel(ch: u8, colors: &[Color], brightness: f32, infinity: bool) -> Vec<Vec<u8>> {
    if colors.is_empty() {
        return Vec::new();
    }
    let fans = (colors.len() - 1) / 16 + 1;
    let mut data = buf(353, &[REPORT, 0x30 + ch]);
    for (i, c) in colors.iter().enumerate() {
        let s = brightness * v2_limit(*c);
        let at = 2 + i * 3;
        data[at..at + 3].copy_from_slice(&[
            (c.0 as f32 * s) as u8,
            (c.2 as f32 * s) as u8,
            (c.1 as f32 * s) as u8,
        ]);
    }
    let (start, short) = if infinity {
        (buf(65, &[REPORT, 0x10, 0x60, 1 + ch / 2, 0x04]), 65)
    } else {
        (buf(353, &[REPORT, 0x10, 0x60, (ch << 4) + fans as u8]), 353)
    };
    vec![
        start,
        data,
        buf(short, &[REPORT, 0x10 + ch, 0x01, 0x02, 0x00, 0x00]),
    ]
}

struct UniHubV2 {
    infinity: bool,
    counts: Vec<usize>,
}

impl Driver for UniHubV2 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for (ch, &n) in self.counts.iter().enumerate() {
            for p in v2_channel(ch as u8, &colors[at..at + n], 1.0, self.infinity) {
                io.write(&p)?;
                io.sleep(5);
            }
            at += n;
        }
        Ok(())
    }
}

fn open_v2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let id = info.device_id("lianli");
    let counts = sized(&id, 4, V2_LEDS);
    let dev = hub(id, d.name, V2_LEDS, &counts);
    Ok(Hid::boxed(
        "lian li uni hub sl v2",
        info.port(api)?,
        UniHubV2 {
            infinity: false,
            counts,
        },
        dev,
    ))
}

fn open_infinity(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let id = info.device_id("lianli");
    let counts = sized(&id, 8, V2_LEDS);
    let dev = hub(id, d.name, V2_LEDS, &counts);
    Ok(Hid::boxed(
        "lian li uni hub sl infinity",
        info.port(api)?,
        UniHubV2 {
            infinity: true,
            counts,
        },
        dev,
    ))
}

/// Strimer L Connect strips: six 24-pin strips of 20 LEDs, then six GPU strips of 27.
pub const STRIMER_STRIPS: [usize; 12] = [20, 20, 20, 20, 20, 20, 27, 27, 27, 27, 27, 27];

/// SetLedsDirect: R, B, G per LED.
pub fn strimer_leds(zone: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = buf(255, &[REPORT, 0x30 | zone]);
    for (i, c) in colors.iter().enumerate() {
        b[2 + 3 * i..5 + 3 * i].copy_from_slice(&[c.0, c.2, c.1]);
    }
    b
}

/// SetMode(Direct, zone, speed 0, brightness 0, direction 0), the Direct mode's defaults.
pub fn strimer_mode(zone: u8) -> Vec<u8> {
    buf(255, &[REPORT, 0x10 | zone, 0x01, 0x02, 0x01, 0x08])
}

pub fn strimer_apply() -> Vec<u8> {
    buf(255, &[REPORT, 0x2C, 0x0F, 0xFF])
}

struct Strimer;

impl Driver for Strimer {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for (z, n) in STRIMER_STRIPS.iter().enumerate() {
            io.write(&strimer_leds(z as u8, &colors[at..at + n]))?;
            io.write(&strimer_mode(z as u8))?;
            at += n;
        }
        io.write(&strimer_apply())
    }

    fn min_interval(&self) -> Duration {
        Duration::from_millis(90)
    }
}

fn strimer_device(id: String, name: &str) -> Device {
    let mut d = line(id, name, DeviceKind::Strip, FANS, 0);
    for (r, n) in STRIMER_STRIPS.iter().enumerate() {
        d.leds
            .extend((0..*n).map(|c| [(c as f32 + 0.5) / *n as f32, (r as f32 + 0.5) / 12.0]));
    }
    d.grid = [27, 12];
    d
}

fn open_strimer(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = strimer_device(info.device_id("lianli"), d.name);
    Ok(Hid::boxed(
        "lian li strimer l connect",
        info.port(api)?,
        Strimer,
        dev,
    ))
}

/// SetMode_StaticColor at 100% brightness: inner ring then outer ring.
pub fn ga2_static(inner: Color, outer: Color) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..16].copy_from_slice(&[
        0x01, 0x83, 0x00, 0x00, 0x00, 0x13, 0x02, 0x03, 0x04, 0x00, inner.0, inner.1, inner.2,
        outer.0, outer.1, outer.2,
    ]);
    b
}

struct GaII;

impl Driver for GaII {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&ga2_static(colors[0], colors[1]))
    }
}

fn open_ga2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info.device_id("lianli"), d.name, DeviceKind::Other, FANS, 2);
    Ok(Hid::boxed(
        "lian li ga ii trinity",
        info.port(api)?,
        GaII,
        dev,
    ))
}

macro_rules! hub {
    ($name:expr, $pid:expr, $open:expr) => {
        Detector {
            name: $name,
            vid: ENE,
            pid: $pid,
            interface: Some(1),
            usage_page: Some(0xFF72),
            usage: Some(0xA1),
            open: $open,
        }
    };
}

macro_rules! ga2 {
    ($name:expr, $pid:expr) => {
        Detector {
            name: $name,
            vid: NUVOTON,
            pid: $pid,
            interface: Some(2),
            usage_page: None,
            usage: None,
            open: open_ga2,
        }
    };
}

pub static DETECTORS: [Detector; 9] = [
    hub!("Lian Li Uni Hub - SL", 0xA100, open_sl),
    hub!("Lian Li Uni Hub - AL", 0xA101, open_al),
    hub!("Lian Li Uni Hub - SL V2", 0xA103, open_v2),
    hub!("Lian Li Uni Hub - AL V2", 0xA104, open_v2),
    hub!("Lian Li Uni Hub - SL V2 v0.5", 0xA105, open_v2),
    hub!("Lian Li Uni Hub - SL Infinity", 0xA102, open_infinity),
    hub!("Lian Li Strimer L Connect", 0xA200, open_strimer),
    ga2!("Lian Li GA II Trinity", 0x7373),
    ga2!("Lian Li GA II Trinity Performance", 0x7371),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Mock, Op, pad};
    use super::*;

    const C1: Color = Color(0x11, 0x22, 0x33);

    // ReadVersion / GetFirmwareVersionString.
    #[test]
    fn firmware_tag() {
        assert_eq!(firmware("LianLi-UNI FAN-SL-v1.8  "), "v1.8");
        assert_eq!(firmware("LianLi-UNI FAN-AL-v1.7"), "v1.7");
    }

    // LianLiUniHubSLController::UpdateMode(Static): activate x4, merge, then colour + mode per channel.
    #[test]
    fn sl_frame() {
        let m = Mock::default();
        let dev = rows("s".into(), "S", DeviceKind::Other, FANS, 4, 4);
        let sl = UniHubSl {
            counts: vec![SL_FANS; 4],
        };
        let mut b = Hid::new("t", Box::new(m.clone()), sl, dev);
        let mut colors = vec![Color::BLACK; 16];
        colors[4] = C1;
        b.push("s", &colors).unwrap();
        let ops: Vec<Op> = m
            .take()
            .into_iter()
            .filter(|o| *o != Op::Sleep(5))
            .collect();
        assert_eq!(ops.len(), 13);
        assert_eq!(ops[0], Op::Write(pad(&[0xE0, 0x10, 0x32, 0x04], 11)));
        assert_eq!(ops[3], Op::Write(pad(&[0xE0, 0x10, 0x32, 0x34], 11)));
        assert_eq!(ops[4], Op::Write(pad(&[0xE0, 0x10, 0x34], 11)));
        let mut col = pad(&[0xE0, 0x31], 194);
        for led in 0..16 {
            col[2 + led * 3..5 + led * 3].copy_from_slice(&[0x11, 0x33, 0x22]);
        }
        assert_eq!(ops[7], Op::Write(col));
        assert_eq!(
            ops[8],
            Op::Write(pad(&[0xE0, 0x11, 0x01, 0x02, 0x00, 0x00], 11))
        );
    }

    #[derive(Clone, Default)]
    struct Sent(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

    impl Reports for Sent {
        fn output(&mut self, report: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().push(report.to_vec());
            Ok(())
        }
    }

    fn hid_writes(m: &Mock) -> Vec<Vec<u8>> {
        m.take()
            .into_iter()
            .filter_map(|o| match o {
                Op::Write(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    // A hub bound to WinUSB gets the very reports a HID write would carry.
    #[test]
    fn winusb_lighting_sends_the_hid_reports_byte_for_byte() {
        let colors: Vec<Color> = (0..320u32)
            .map(|i| Color(i as u8, (i * 7) as u8, (i * 13) as u8))
            .collect();
        for (pid, product, leds) in [
            (0xA100, "LianLi-UNI FAN-SL-v1.8", SL_FANS),
            (0xA101, "LianLi-UNI FAN-AL-v1.7", AL_LEDS),
        ] {
            let m = Mock::default();
            let counts = vec![leds; 4];
            let dev = hub("hid:x".into(), "X", leds, &counts);
            let mut hid: Box<dyn Backend> = if pid == 0xA100 {
                Box::new(Hid::new("t", Box::new(m.clone()), UniHubSl { counts }, dev))
            } else {
                Box::new(Hid::new("t", Box::new(m.clone()), UniHubAl { counts }, dev))
            };
            let id = format!("usb:lianli-0cf2{pid:04x}-winusb");
            let sent = Sent::default();
            let mut usb = over_usb(pid, product, id.clone(), "X", Box::new(sent.clone())).unwrap();
            assert_eq!(usb.devices()[0].leds.len(), leds * 4);
            let frame = &colors[..leds * 4];
            hid.push("hid:x", frame).unwrap();
            usb.push(&id, frame).unwrap();
            let usb_sent = sent.0.lock().unwrap().clone();
            assert!(!usb_sent.is_empty());
            assert_eq!(usb_sent, hid_writes(&m), "{pid:04x}");
        }
        let none =
            |pid, product| over_usb(pid, product, "u".into(), "X", Box::new(Sent::default()));
        assert!(none(0xA100, "LianLi-UNI FAN-SL-v1.0").is_none());
        assert!(none(0xA101, "LianLi-UNI FAN-AL-v1.0").is_none());
        assert!(none(0xA103, "LianLi-UNI FAN-SL-v1.8").is_none());
    }

    #[test]
    fn a_set_length_sizes_a_channel() {
        crate::lengths::set_one("hid:lianli-test", 0, 2);
        crate::lengths::set_one("hid:lianli-test", 3, 0);
        let counts = sized("hid:lianli-test", 4, SL_FANS);
        assert_eq!(counts, [2, 4, 4, 0]);
        let dev = hub("s".into(), "S", SL_FANS, &counts);
        assert_eq!(dev.leds.len(), 10);
        let m = Mock::default();
        let mut b = Hid::new("t", Box::new(m.clone()), UniHubSl { counts }, dev);
        b.push("s", &[C1; 10]).unwrap();
        let ops: Vec<Op> = m
            .take()
            .into_iter()
            .filter(|o| *o != Op::Sleep(5))
            .collect();
        assert_eq!(ops[0], Op::Write(pad(&[0xE0, 0x10, 0x32, 0x02], 11)));
        assert_eq!(ops[3], Op::Write(pad(&[0xE0, 0x10, 0x32, 0x30], 11)));
        let Op::Write(col) = &ops[5] else { panic!() };
        assert_eq!(col[2 + 32 * 3..5 + 32 * 3], [0, 0, 0]);
    }

    // LianLiUniHubALController::SetChannelLEDs: fan and edge halves, R B G, white limiter.
    #[test]
    fn al_channel_reports() {
        let mut colors = vec![Color::BLACK; 80];
        colors[0] = C1;
        colors[8] = Color(200, 200, 200);
        colors[20] = Color(1, 2, 3);
        let p = al_channel(2, &colors, 1.0);
        assert_eq!(p.len(), 6);
        assert_eq!(p[0], pad(&[0xE0, 0x10, 0x40, 0x03, 0x04], 65));
        let mut fan = pad(&[0xE0, 0x34, 0x11, 0x33, 0x22], 146);
        fan[2 + 24..2 + 27].copy_from_slice(&[1, 3, 2]);
        assert_eq!(p[1], fan);
        assert_eq!(p[2], pad(&[0xE0, 0x14, 0x01, 0x02, 0x00, 0x00], 65));
        assert_eq!(p[4], pad(&[0xE0, 0x35, 153, 153, 153], 146));
        assert_eq!(p[5], pad(&[0xE0, 0x15, 0x01, 0x02, 0x00, 0x00], 65));
        // the 37/50 default brightness truncates
        let dim = al_channel(0, &[Color(100, 0, 0)], 37.0 / 50.0);
        assert_eq!(dim[1][2], 74);
        assert_eq!(dim[0][4], 1);
    }

    // LianLiUniHubSLV2Controller::SetChannelLEDs and brightnessLimit; the SL Infinity variant.
    #[test]
    fn v2_channel_reports() {
        let mut colors = vec![Color::BLACK; 96];
        colors[0] = Color(255, 255, 255);
        colors[95] = C1;
        let p = v2_channel(1, &colors, 1.0, false);
        assert_eq!(p[0], pad(&[0xE0, 0x10, 0x60, 0x16], 353));
        // 255 * 460 / 765 = 153.33
        assert_eq!(p[1][..5], [0xE0, 0x31, 153, 153, 153]);
        assert_eq!(p[1][2 + 95 * 3..5 + 95 * 3], [0x11, 0x33, 0x22]);
        assert_eq!(p[2], pad(&[0xE0, 0x11, 0x01, 0x02, 0x00, 0x00], 353));
        let inf = v2_channel(5, &colors, 1.0, true);
        assert_eq!(inf[0], pad(&[0xE0, 0x10, 0x60, 0x03, 0x04], 65));
        assert_eq!(inf[1].len(), 353);
        assert_eq!(inf[2], pad(&[0xE0, 0x15, 0x01, 0x02, 0x00, 0x00], 65));
    }

    // RGBController_LianLiStrimerLConnect::DeviceUpdateLEDs: per zone colours and mode, then apply.
    #[test]
    fn strimer_frame() {
        assert_eq!(strimer_leds(11, &[C1])[..5], [0xE0, 0x3B, 0x11, 0x33, 0x22]);
        assert_eq!(
            strimer_mode(3),
            pad(&[0xE0, 0x13, 0x01, 0x02, 0x01, 0x08], 255)
        );
        assert_eq!(strimer_apply(), pad(&[0xE0, 0x2C, 0x0F, 0xFF], 255));
        let m = Mock::default();
        let dev = strimer_device("s".into(), "S");
        assert_eq!(dev.leds.len(), 282);
        let mut b = Hid::new("t", Box::new(m.clone()), Strimer, dev);
        b.push("s", &vec![C1; 282]).unwrap();
        // the 90 ms limit drops a frame that comes too soon
        b.push("s", &vec![Color(1, 1, 1); 282]).unwrap();
        let ops = m.take();
        assert_eq!(ops.len(), 25);
        assert_eq!(ops[12], Op::Write(strimer_leds(6, &[C1; 27])));
    }

    // LianLiGAIITrinityController::SetMode_StaticColor(B_100, inner, outer).
    #[test]
    fn ga2_report() {
        let p = ga2_static(C1, Color(4, 5, 6));
        assert_eq!(
            p.to_vec(),
            pad(
                &[
                    0x01, 0x83, 0, 0, 0, 0x13, 0x02, 0x03, 0x04, 0x00, 0x11, 0x22, 0x33, 4, 5, 6
                ],
                64
            )
        );
    }
}
