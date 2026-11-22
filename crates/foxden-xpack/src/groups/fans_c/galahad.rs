//! Lian Li Galahad II Trinity and Galahad II LCD AIOs.

use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Driver, Io, Proto, drain, on_i, open_io};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_fan::FanBackend;
use foxden_hid::port::Port;
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const NUVOTON: u16 = 0x0416;
pub const LCD_PID: u16 = 0x7395;
const FAMILY: &str = "lianli-galahad2";

pub const SMALL: u8 = 0x01;
pub const SMALL_LEN: usize = 64;
pub const LARGE: u8 = 0x02;
pub const LARGE_LEN: usize = 1024;

pub const GET_INFO: u8 = 0x81;
pub const PUMP_COLORS: u8 = 0x83;
pub const FAN_COLORS: u8 = 0x85;
pub const PUMP_SPEED: u8 = 0x8A;
pub const FAN_SPEED: u8 = 0x8B;
pub const PER_LED: u8 = 0x14;

pub const PUMP_LEDS: usize = 12;
/// The LCD's fan channel holds up to 50 LEDs.
pub const FAN_LEDS: usize = 50;
/// Per-LED pump data starts after eight unused LED slots.
const PUMP_SKIP: usize = 24;
/// Acks of these are never status replies.
const OTHER_COMMANDS: [u8; 5] = [PUMP_COLORS, FAN_COLORS, PUMP_SPEED, FAN_SPEED, PER_LED];
/// The plugin's default shutdown colour.
const SHUTDOWN: Color = Color(0x00, 0x9B, 0xDE);
/// Pump speed sent when a fan write first takes the AIO into software mode.
const PUMP_TAKEOVER: f32 = 1.0;

/// `[1, cmd, 0, seq hi, seq lo, len, data..]`, 64 bytes.
pub fn small(cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; SMALL_LEN];
    p[..6].copy_from_slice(&[SMALL, cmd, 0, 0, 0, data.len() as u8]);
    p[6..6 + data.len()].copy_from_slice(data);
    p
}

/// `[2, cmd, total u32 BE, seq u24 BE, len u16 BE, data..]`, 1024 bytes.
pub fn large(cmd: u8, data: &[u8], total: u32, seq: u32) -> Vec<u8> {
    let mut p = vec![0u8; LARGE_LEN];
    p[0] = LARGE;
    p[1] = cmd;
    p[2..6].copy_from_slice(&total.to_be_bytes());
    p[6..9].copy_from_slice(&seq.to_be_bytes()[1..]);
    p[9..11].copy_from_slice(&(data.len() as u16).to_be_bytes());
    p[11..11 + data.len()].copy_from_slice(data);
    p
}

/// Static fan colour: effect 3, brightness 4, speed 0, colour, LED count last.
pub fn fan_static(c: Color, leds: u8) -> Vec<u8> {
    let mut d = vec![0u8; 20];
    d[..6].copy_from_slice(&[0x03, 0x04, 0x00, c.0, c.1, c.2]);
    d[19] = leds;
    d
}

/// Static pump colour for one zone (0 inner, 1 outer).
pub fn pump_static(zone: u8, c: Color) -> Vec<u8> {
    let mut d = vec![0u8; 19];
    d[..7].copy_from_slice(&[zone, 0x03, 0x04, 0x00, c.0, c.1, c.2]);
    d
}

/// Per-LED pump payload: zone 0, eight empty slots, then the ring in reverse slot order.
pub fn pump_payload(colors: &[Color]) -> Vec<u8> {
    let mut d = vec![0u8; 1 + PUMP_SKIP + PUMP_LEDS * 3];
    for (i, c) in colors.iter().take(PUMP_LEDS).enumerate() {
        let at = 1 + PUMP_SKIP + (PUMP_LEDS - 1 - i) * 3;
        d[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    d
}

/// Per-LED fan payload: zone 1, then R G B per LED.
pub fn fan_payload(colors: &[Color]) -> Vec<u8> {
    let mut d = vec![0x01];
    for c in colors {
        d.extend_from_slice(&[c.0, c.1, c.2]);
    }
    d
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub fan_rpm: u32,
    pub pump_rpm: u32,
    pub coolant: Option<f32>,
}

/// The 0x81 reply: fan and pump rpm big endian at 6 and 8; coolant at 11 (+ tenths at 12) when 10 is 1.
pub fn parse_status(b: &[u8]) -> Option<Status> {
    if b.len() < 13 {
        return None;
    }
    Some(Status {
        fan_rpm: u16::from_be_bytes([b[6], b[7]]) as u32,
        pump_rpm: u16::from_be_bytes([b[8], b[9]]) as u32,
        coolant: (b[10] == 1).then(|| b[11] as f32 + b[12] as f32 / 10.0),
    })
}

struct LcdLights;

impl LcdLights {
    fn send(io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let (pump, fans) = colors.split_at(PUMP_LEDS.min(colors.len()));
        for d in [pump_payload(pump), fan_payload(fans)] {
            io.write(&large(PER_LED, &d, d.len() as u32, 0), LARGE_LEN)?;
        }
        Ok(())
    }
}

impl Proto for LcdLights {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        let black = Color(0, 0, 0);
        io.write(
            &small(FAN_COLORS, &fan_static(black, FAN_LEDS as u8)),
            SMALL_LEN,
        )?;
        io.write(&small(PUMP_COLORS, &pump_static(0, black)), SMALL_LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        Self::send(io, colors)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = Self::send(io, &[SHUTDOWN; PUMP_LEDS + FAN_LEDS]);
    }
}

fn lcd_device(id: String, name: &str) -> Device {
    let mut d = Device::grid(
        &id,
        name,
        DeviceKind::Other,
        [0.06, 0.24, 0.52, 0.24],
        FAN_LEDS as u32,
        2,
    );
    d.leds = (0..PUMP_LEDS)
        .map(|i| [(i as f32 + 0.5) / PUMP_LEDS as f32, 0.25])
        .chain((0..FAN_LEDS).map(|i| [(i as f32 + 0.5) / FAN_LEDS as f32, 0.75]))
        .collect();
    d
}

fn open_lcd(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = lcd_device(info.device_id(FAMILY), d.name);
    Ok(Driver::new(
        open_io(api, info)?,
        dev,
        "lian li galahad ii lcd",
        LcdLights,
    )
    .boxed())
}

pub static DETECTORS: [Detector; 1] = [on_i(
    "Lian Li Galahad II LCD",
    NUVOTON,
    LCD_PID,
    1,
    open_lcd,
)];

struct Model {
    pid: u16,
    name: &'static str,
    interface: i32,
    coolant: bool,
}

static MODELS: [Model; 2] = [
    Model {
        pid: 0x7373,
        name: "Lian Li Galahad II Trinity",
        interface: 2,
        coolant: false,
    },
    Model {
        pid: LCD_PID,
        name: "Lian Li Galahad II LCD",
        interface: 1,
        coolant: true,
    },
];

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|m| (NUVOTON, m.pid, m.name)).collect()
}

const PUMP: usize = 0;
const FAN: usize = 1;

struct Hub {
    dev: Port,
    id: String,
    name: &'static str,
    coolant: bool,
    status: Option<Status>,
    read_at: Option<Instant>,
    duty: [Option<u8>; 2],
}

impl Hub {
    fn refresh(&mut self) {
        if self
            .read_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.read_at = Some(Instant::now());
        drain(&mut self.dev, SMALL_LEN);
        if Io::write(&mut self.dev, &[SMALL, GET_INFO], SMALL_LEN).is_err() {
            self.status = None;
            return;
        }
        let end = Instant::now() + Duration::from_millis(300);
        let mut first = None;
        while Instant::now() < end {
            let left = end.saturating_duration_since(Instant::now()).as_millis() as i32;
            let Ok(r) = Io::read(&mut self.dev, SMALL_LEN, left.max(1)) else {
                break;
            };
            if r.first() != Some(&SMALL) {
                continue;
            }
            if r.get(1) == Some(&GET_INFO) {
                first = Some(r);
                break;
            }
            if !r.get(1).is_some_and(|c| OTHER_COMMANDS.contains(c)) {
                first.get_or_insert(r);
            }
        }
        self.status = first.as_deref().and_then(parse_status);
    }
    fn speed(&mut self, ch: usize, p: u8) -> Result<(), String> {
        let pkt = if ch == PUMP {
            small(PUMP_SPEED, &[0x00, p])
        } else {
            small(FAN_SPEED, &[0x00, p])
        };
        Io::write(&mut self.dev, &pkt, SMALL_LEN)?;
        self.duty[ch] = Some(p);
        Ok(())
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.into()
    }
    fn channels(&self) -> Vec<Channel> {
        vec![Channel::pump("Pump"), Channel::fan("Fan")]
    }
    fn sensors(&self) -> Vec<String> {
        if self.coolant {
            vec!["Liquid Temperature".into()]
        } else {
            Vec::new()
        }
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        let s = self.status?;
        Some(if ch == PUMP { s.pump_rpm } else { s.fan_rpm })
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        let p = percent(duty);
        if self.duty[ch] == Some(p) {
            return Ok(());
        }
        // the pump command also clears the motherboard PWM flag
        if ch == FAN && self.duty[PUMP].is_none() {
            self.speed(PUMP, percent(PUMP_TAKEOVER))?;
        }
        self.speed(ch, p)
    }
    fn release(&mut self, _ch: usize) -> Result<(), String> {
        Ok(())
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        self.refresh();
        self.status?.coolant
    }
}

/// One hub per attached AIO, on the interface its plugin validates.
pub fn fans(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in foxden_hid::device_list(api).iter() {
        let info = Info::from_hidapi(raw);
        let Some(m) = MODELS
            .iter()
            .find(|m| info.vid == NUVOTON && info.pid == m.pid && info.interface == m.interface)
        else {
            continue;
        };
        if !seen.insert(info.key()) {
            continue;
        }
        let Ok(dev) = info.port(api) else {
            continue;
        };
        out.push(backend_on(
            &info,
            Hub {
                dev,
                id: info.device_id(FAMILY).replacen("hid:", "fan:", 1),
                name: m.name,
                coolant: m.coolant,
                status: None,
                read_at: None,
                duty: [None; 2],
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // sendSmallPacket / setPumpSpeed / setFanSpeed.
    #[test]
    fn small_packets() {
        let p = small(PUMP_SPEED, &[0x00, 60]);
        assert_eq!(p.len(), 64);
        assert_eq!(p[..8], [0x01, 0x8A, 0x00, 0x00, 0x00, 0x02, 0x00, 60]);
        assert_eq!(fan_static(Color(1, 2, 3), 50)[..6], [3, 4, 0, 1, 2, 3]);
        assert_eq!(fan_static(Color(0, 0, 0), 50)[19], 50);
        assert_eq!(pump_static(1, Color(4, 5, 6))[..7], [1, 3, 4, 0, 4, 5, 6]);
    }

    // sendLargePacket / setPumpPerLED / setFansPerLED.
    #[test]
    fn large_packets() {
        let mut ring = [Color(0, 0, 0); PUMP_LEDS];
        ring[0] = Color(9, 8, 7);
        let d = pump_payload(&ring);
        assert_eq!(d.len(), 61);
        assert_eq!(d[0], 0);
        assert_eq!(d[1 + 24 + 33..], [9, 8, 7]);
        let p = large(PER_LED, &d, 61, 0x010203);
        assert_eq!(p.len(), 1024);
        assert_eq!(p[..11], [0x02, 0x14, 0, 0, 0, 61, 0x01, 0x02, 0x03, 0, 61]);
        assert_eq!(fan_payload(&[Color(1, 2, 3)]), [1, 1, 2, 3]);
    }

    // fetchRPM.
    #[test]
    fn status_reply() {
        let mut b = [0u8; 64];
        b[..13].copy_from_slice(&[1, 0x81, 0, 0, 0, 0, 0x03, 0xE8, 0x0A, 0x8C, 1, 31, 5]);
        let s = parse_status(&b).unwrap();
        assert_eq!(s.fan_rpm, 1000);
        assert_eq!(s.pump_rpm, 2700);
        assert_eq!(s.coolant, Some(31.5));
        b[10] = 0;
        assert_eq!(parse_status(&b).unwrap().coolant, None);
    }
}
