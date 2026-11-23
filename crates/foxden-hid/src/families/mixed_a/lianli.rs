// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::Duration;

use foxden_core::{Backend, Color, Device, DeviceKind, Hardware, HardwareMode, HardwareSetting};
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

/// Each channel at the user's length, in whole fans of `per_fan` LEDs.
fn sized(id: &str, channels: usize, max: usize, per_fan: usize) -> Vec<usize> {
    (0..channels)
        .map(|i| {
            let name = format!("Channel {}", i + 1);
            crate::lengths::length_step(id, i, &name, max, 0, max, per_fan)
        })
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

const HUB_SPEEDS: [u8; 5] = [0x02, 0x01, 0x00, 0xFF, 0xFE];
const HUB_BRIGHTNESS: [u8; 5] = [0x08, 0x03, 0x02, 0x01, 0x00];
/// Mode id, name, mode byte, colours, speed, direction; the AL takes the first two.
const HUB_EFFECTS: [(&str, &str, u8, usize, bool, bool); 4] = [
    ("static", "Static", 0x01, 1, false, false),
    ("breathing", "Breathing", 0x02, 1, true, false),
    ("color_cycle", "Colour cycle", 0x04, 0, true, false),
    ("rainbow", "Rainbow", 0x05, 0, true, true),
];

fn hub_hardware(al: bool) -> Hardware {
    let n = if al { 2 } else { HUB_EFFECTS.len() };
    let modes = HUB_EFFECTS[..n]
        .iter()
        .map(|&(id, name, _, colors, speed, turns)| HardwareMode {
            speed: speed.then_some([0, 4]),
            brightness: Some([0, 4]),
            colors,
            directions: if turns {
                vec!["right".into(), "left".into()]
            } else {
                Vec::new()
            },
            ..HardwareMode::new(id, name)
        })
        .collect();
    Hardware { modes }
}

/// The colour, its brightness scale, and the mode packet's effect, speed, direction and brightness bytes.
fn hub_effect(al: bool, s: &HardwareSetting) -> Result<(Color, f32, [u8; 4]), String> {
    let (_, f) = hub_hardware(al).fit(s)?;
    let mode = HUB_EFFECTS
        .iter()
        .find(|e| e.0 == s.mode)
        .ok_or("unknown mode")?
        .2;
    let c = f.colors.first().copied().unwrap_or(Color::BLACK);
    let commit = [
        mode,
        HUB_SPEEDS[f.speed as usize],
        f.direction as u8,
        HUB_BRIGHTNESS[f.brightness as usize],
    ];
    Ok((c, f32::from(f.brightness) / 4.0, commit))
}

fn send_all(io: &mut dyn Io, packets: Vec<Vec<u8>>) -> Result<(), String> {
    for p in packets {
        io.write(&p)?;
        io.sleep(5);
    }
    Ok(())
}

/// Uni Hub SL: four fans of 16 LEDs on each of four channels.
pub const SL_FANS: usize = 4;
pub const SL_FAN_LEDS: usize = 16;
pub const SL_LEDS: usize = SL_FANS * SL_FAN_LEDS;

/// SL channels at the user's length; counts saved before were whole fans.
fn sl_counts(id: &str) -> Vec<usize> {
    for ch in 0..4 {
        crate::lengths::fans_to_leds(id, ch, SL_FANS, SL_FAN_LEDS);
    }
    sized(id, 4, SL_LEDS, SL_FAN_LEDS)
}

/// LianLiUniHubSLController::SendActivate.
pub fn sl_activate(ch: u8, fans: u8) -> Vec<u8> {
    buf(11, &[REPORT, 0x10, 0x32, 0x10 * ch + fans])
}

/// SendMerge, not merged.
pub fn sl_merge() -> Vec<u8> {
    buf(11, &[REPORT, 0x10, 0x34])
}

/// SendColor: 16 LEDs per fan, R B G order, laid out as the SL V2 and AL colour packets.
pub fn sl_color(ch: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = buf(2 + SL_LEDS * 3, &[REPORT, 0x30 + ch]);
    for (i, c) in colors.iter().take(SL_LEDS).enumerate() {
        b[2 + i * 3..5 + i * 3].copy_from_slice(&[c.0, c.2, c.1]);
    }
    b
}

/// SendMode for Static: speed 0 -> 0x02, direction left -> 0x00, brightness 4 -> 0x00.
pub fn sl_mode(ch: u8) -> Vec<u8> {
    buf(11, &[REPORT, 0x10 + ch, 0x01, 0x02, 0x00, 0x00])
}

/// The direct frame's packets, every LED at `c` and each channel's mode packet carrying `commit`.
pub fn sl_effect(counts: &[usize], c: Color, commit: [u8; 4]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = counts
        .iter()
        .enumerate()
        .map(|(ch, &n)| sl_activate(ch as u8, n.div_ceil(SL_FAN_LEDS) as u8))
        .collect();
    out.push(sl_merge());
    for (ch, &n) in counts.iter().enumerate() {
        out.push(sl_color(ch as u8, &vec![c; n]));
        let mut mode = sl_mode(ch as u8);
        mode[2..6].copy_from_slice(&commit);
        out.push(mode);
    }
    out
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
            send(io, sl_activate(ch as u8, n.div_ceil(SL_FAN_LEDS) as u8))?;
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

    fn hardware(&self) -> Option<Hardware> {
        Some(hub_hardware(false))
    }

    fn set_hardware(&mut self, io: &mut dyn Io, s: &HardwareSetting) -> Result<(), String> {
        let (c, k, commit) = hub_effect(false, s)?;
        let scale = |v: u8| (f32::from(v) * k) as u8;
        let c = Color(scale(c.0), scale(c.1), scale(c.2));
        send_all(io, sl_effect(&self.counts, c, commit))
    }
}

fn open_sl(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    if firmware(&info.product) != "v1.8" {
        return Err(format!("{} firmware is not v1.8", d.name));
    }
    let id = info.device_id("lianli");
    let counts = sl_counts(&id);
    let dev = hub(id, d.name, SL_LEDS, &counts);
    Ok(Hid::boxed(
        "lian li uni hub sl",
        info.port(api)?,
        UniHubSl { counts },
        dev,
    ))
}

/// Uni Hub AL: four fans per channel, each 8 fan LEDs then 12 edge LEDs.
pub const AL_FAN_LEDS: usize = 20;
pub const AL_LEDS: usize = 4 * AL_FAN_LEDS;

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
        let (f, m) = (i / AL_FAN_LEDS, i % AL_FAN_LEDS);
        let px = [scale(c.0), scale(c.2), scale(c.1)];
        if m < 8 {
            let at = (m + f * 8) * 3;
            fan[at..at + 3].copy_from_slice(&px);
        } else {
            let at = (m - 8 + f * 12) * 3;
            edge[at..at + 3].copy_from_slice(&px);
        }
    }
    let fans = colors.len().div_ceil(AL_FAN_LEDS);
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

/// One channel's direct packets at `c` and `brightness`, both commits carrying `commit`.
pub fn al_effect(ch: u8, n: usize, c: Color, brightness: f32, commit: [u8; 4]) -> Vec<Vec<u8>> {
    let mut out = al_channel(ch, &vec![c; n], brightness);
    for at in [2, 5] {
        if let Some(p) = out.get_mut(at) {
            p[2..6].copy_from_slice(&commit);
        }
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

    fn hardware(&self) -> Option<Hardware> {
        Some(hub_hardware(true))
    }

    fn set_hardware(&mut self, io: &mut dyn Io, s: &HardwareSetting) -> Result<(), String> {
        let (c, k, commit) = hub_effect(true, s)?;
        for (ch, &n) in self.counts.iter().enumerate() {
            send_all(io, al_effect(ch as u8, n, c, k, commit))?;
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
    let counts = sized(&id, 4, AL_LEDS, AL_FAN_LEDS);
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
            let counts = sl_counts(&id);
            let dev = hub(id, name, SL_LEDS, &counts);
            Some(Hid::boxed(
                "lian li uni hub sl",
                io,
                UniHubSl { counts },
                dev,
            ))
        }
        (0xA101, f) if f.starts_with("v1.7") => {
            let counts = sized(&id, 4, AL_LEDS, AL_FAN_LEDS);
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
pub const V2_FAN_LEDS: usize = 16;
pub const V2_LEDS: usize = 6 * V2_FAN_LEDS;

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
    let fans = colors.len().div_ceil(V2_FAN_LEDS);
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

/// One channel's direct packets at `c` and `brightness`, the commit carrying `commit`.
pub fn v2_effect(
    ch: u8,
    n: usize,
    c: Color,
    brightness: f32,
    infinity: bool,
    commit: [u8; 4],
) -> Vec<Vec<u8>> {
    let mut out = v2_channel(ch, &vec![c; n], brightness, infinity);
    if let Some(p) = out.last_mut() {
        p[2..6].copy_from_slice(&commit);
    }
    out
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

    fn hardware(&self) -> Option<Hardware> {
        Some(hub_hardware(false))
    }

    fn set_hardware(&mut self, io: &mut dyn Io, s: &HardwareSetting) -> Result<(), String> {
        let (c, k, commit) = hub_effect(false, s)?;
        for (ch, &n) in self.counts.iter().enumerate() {
            send_all(io, v2_effect(ch as u8, n, c, k, self.infinity, commit))?;
        }
        Ok(())
    }
}

fn open_v2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let id = info.device_id("lianli");
    let counts = sized(&id, 4, V2_LEDS, V2_FAN_LEDS);
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
    let counts = sized(&id, 8, V2_LEDS, V2_FAN_LEDS);
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
        let dev = rows("s".into(), "S", DeviceKind::Other, FANS, SL_LEDS, 4);
        let sl = UniHubSl {
            counts: vec![SL_LEDS; 4],
        };
        let mut b = Hid::new("t", Box::new(m.clone()), sl, dev);
        let mut colors = vec![Color::BLACK; 4 * SL_LEDS];
        colors[64] = C1;
        colors[65] = Color(1, 2, 3);
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
        let mut col = pad(&[0xE0, 0x31, 0x11, 0x33, 0x22, 1, 3, 2], 194);
        assert_eq!(ops[7], Op::Write(col.clone()));
        col[1] = 0x30;
        col[2..8].fill(0);
        assert_eq!(ops[5], Op::Write(col));
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
            (0xA100, "LianLi-UNI FAN-SL-v1.8", SL_LEDS),
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
        crate::lengths::set_one("hid:lianli-test", 1, 40);
        let counts = sl_counts("hid:lianli-test");
        assert_eq!(counts, [32, 48, 64, 0]);
        let dev = hub("s".into(), "S", SL_LEDS, &counts);
        assert_eq!(dev.leds.len(), 144);
        let m = Mock::default();
        let mut b = Hid::new("t", Box::new(m.clone()), UniHubSl { counts }, dev);
        b.push("s", &[C1; 144]).unwrap();
        let ops: Vec<Op> = m
            .take()
            .into_iter()
            .filter(|o| *o != Op::Sleep(5))
            .collect();
        assert_eq!(ops[0], Op::Write(pad(&[0xE0, 0x10, 0x32, 0x02], 11)));
        assert_eq!(ops[3], Op::Write(pad(&[0xE0, 0x10, 0x32, 0x30], 11)));
        let Op::Write(col) = &ops[5] else { panic!() };
        assert_eq!(col[2 + 31 * 3..5 + 31 * 3], [0x11, 0x33, 0x22]);
        assert_eq!(col[2 + 32 * 3..5 + 32 * 3], [0, 0, 0]);
        let zones: Vec<_> = crate::lengths::resizable()
            .into_iter()
            .filter(|z| z.device == "hid:lianli-test")
            .collect();
        assert!(
            zones
                .iter()
                .all(|z| (z.max, z.step) == (SL_LEDS, SL_FAN_LEDS))
        );
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

    fn writes(ops: Vec<Op>) -> Vec<Vec<u8>> {
        ops.into_iter()
            .filter_map(|o| match o {
                Op::Write(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn hub_effect_bytes() {
        let mut s = HardwareSetting::new("breathing");
        s.speed = Some(4);
        s.brightness = Some(2);
        s.colors = vec![C1];
        assert_eq!(
            hub_effect(false, &s).unwrap(),
            (C1, 0.5, [0x02, 0xFE, 0x00, 0x02])
        );
        s.speed = Some(0);
        s.brightness = Some(0);
        assert_eq!(hub_effect(true, &s).unwrap().2, [0x02, 0x02, 0x00, 0x08]);
        let mut r = HardwareSetting::new("rainbow");
        r.direction = Some("left".into());
        assert_eq!(
            hub_effect(false, &r).unwrap(),
            (Color::BLACK, 1.0, [0x05, 0x00, 0x01, 0x00])
        );
        assert!(hub_effect(true, &r).is_err());
        let st = HardwareSetting::new("static");
        assert_eq!(hub_effect(false, &st).unwrap().2, [0x01, 0x02, 0x00, 0x00]);
        assert_eq!(
            hub_effect(false, &HardwareSetting::new("color_cycle"))
                .unwrap()
                .2,
            [0x04, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn sl_effect_then_direct_again() {
        let m = Mock::default();
        let dev = rows("s".into(), "S", DeviceKind::Other, FANS, SL_LEDS, 4);
        let sl = UniHubSl {
            counts: vec![SL_LEDS, 16, 0, 0],
        };
        let mut b = Hid::new("t", Box::new(m.clone()), sl, dev);
        let colors = vec![C1; 4 * SL_LEDS];
        b.push("s", &colors).unwrap();
        let direct = writes(m.take());
        let mut s = HardwareSetting::new("static");
        s.brightness = Some(2);
        s.colors = vec![Color(0x40, 0x80, 0xFF)];
        b.set_hardware("s", Some(&s)).unwrap();
        let fx = writes(m.take());
        assert_eq!(fx.len(), 4 + 1 + 8);
        assert_eq!(fx[..5], direct[..5]);
        assert_eq!(fx[1], pad(&[0xE0, 0x10, 0x32, 0x11], 11));
        let mut col = pad(&[0xE0, 0x31], 194);
        for i in 0..16 {
            col[2 + i * 3..5 + i * 3].copy_from_slice(&[0x20, 0x7F, 0x40]);
        }
        assert_eq!(fx[7], col);
        assert_eq!(fx[8], pad(&[0xE0, 0x11, 0x01, 0x02, 0x00, 0x02], 11));
        assert_eq!(fx[12], pad(&[0xE0, 0x13, 0x01, 0x02, 0x00, 0x02], 11));
        b.set_hardware("s", None).unwrap();
        b.push("s", &colors).unwrap();
        assert_eq!(writes(m.take()), direct);
    }

    #[test]
    fn al_and_v2_effects() {
        let commit = [0x02, 0x00, 0x00, 0x01];
        let al = al_effect(1, 40, Color(200, 200, 200), 0.75, commit);
        assert_eq!(al.len(), 6);
        assert_eq!(
            al[..2],
            al_channel(1, &[Color(200, 200, 200); 40], 0.75)[..2]
        );
        assert_eq!(al[1][2..5], [114, 114, 114]);
        assert_eq!(al[2], pad(&[0xE0, 0x12, 0x02, 0x00, 0x00, 0x01], 65));
        assert_eq!(al[5], pad(&[0xE0, 0x13, 0x02, 0x00, 0x00, 0x01], 65));
        assert!(al_effect(0, 0, C1, 1.0, commit).is_empty());

        let v2 = v2_effect(2, 32, C1, 1.0, false, commit);
        assert_eq!(v2[..2], v2_channel(2, &[C1; 32], 1.0, false)[..2]);
        assert_eq!(v2[2], pad(&[0xE0, 0x12, 0x02, 0x00, 0x00, 0x01], 353));
        let inf = v2_effect(3, 16, C1, 0.5, true, commit);
        assert_eq!(inf[0], pad(&[0xE0, 0x10, 0x60, 0x02, 0x04], 65));
        assert_eq!(inf[1][2..5], [0x08, 0x19, 0x11]);
        assert_eq!(inf[2], pad(&[0xE0, 0x13, 0x02, 0x00, 0x00, 0x01], 65));
        assert_eq!(
            UniHubAl { counts: vec![] }.hardware().unwrap().modes.len(),
            2
        );
    }
}
