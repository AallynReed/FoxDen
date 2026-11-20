//! Lian Li Uni Fan controllers SL Infinity, SL V2 and AL V2, fan side (protocol learned from SignalRGB's LianLi_SL_Infinity.js, Lian_Li_Uni_Fans_SL_V2.js and Lian_Li_Uni_Fans_AL_V2.js).

use super::usb::{self, Shared};
use crate::fan::{Channel, FanHub, backend_on, percent};
use foxden_fan::FanBackend;
use foxden_hid::port::Port;
use foxden_hid::registry::Info;
use hidapi::HidApi;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ENE: u16 = 0x0CF2;
const REPORT: u8 = 0xE0;
/// The collection FoxDen's lighting uses too (Validate: interface 1).
const INTERFACE: i32 = 1;
const USAGE_PAGE: u16 = 0xFF72;
const CHANNELS: usize = 4;
const FAMILY: &str = "lianli-unifan";

/// Fan mode register: one software bit and one PWM bit per channel.
pub const FAN_MODE: u8 = 0x62;
pub const APPLY: u8 = 0x50;
pub const SET_SPEED: u8 = 0x20;

/// SL Infinity: feature reports of 6 bytes, rpm from an input report of 65.
pub mod inf {
    pub const FEATURE_LEN: usize = 6;
    pub const RPM_LEN: usize = 65;
    pub const RPM_REQUEST: [u8; 3] = [0xE0, 0x50, 0x00];
    /// setFanPercent never sends less.
    pub const MIN_PERCENT: u8 = 20;
    /// setFanMode pauses this long before each mode write.
    pub const MODE_PAUSE_MS: u64 = 20;
}

/// SL V2 / AL V2: output reports; rpm only over a control transfer (hybrid).
pub mod v2 {
    pub const MODE_LEN: usize = 353;
    pub const SPEED_LEN: usize = 32;
    pub const RPM_READ: [u16; 4] = [0xD800, 0xD802, 0xD804, 0xD806];
    pub const RPM_LEN: usize = 65;
    pub const MODE_PAUSE_MS: u64 = 10;
}

/// `E0 10 62`: SL Infinity's mode byte for one channel.
pub fn inf_mode(ch: usize, pwm: bool) -> [u8; 4] {
    [
        REPORT,
        0x10,
        FAN_MODE,
        (1u8 << (ch + 4)) | ((pwm as u8) << ch),
    ]
}

/// SL Infinity speed: `E0 2x 00 percent`, at least 20.
pub fn inf_speed(ch: usize, duty: f32) -> [u8; 4] {
    [
        REPORT,
        SET_SPEED | ch as u8,
        0,
        percent(duty).max(inf::MIN_PERCENT),
    ]
}

/// Rpm of four channels from the 0x50 reply, big endian from byte 1.
pub fn inf_rpms(b: &[u8]) -> [Option<u32>; CHANNELS] {
    let mut out = [None; CHANNELS];
    for (f, o) in out.iter_mut().enumerate() {
        if let (Some(hi), Some(lo)) = (b.get(1 + 2 * f), b.get(2 + 2 * f)) {
            *o = Some(((*hi as u32) << 8) | *lo as u32);
        }
    }
    out
}

/// `E0 10 62`: SL V2 / AL V2 software (0x10 << ch) or motherboard (0x11 << ch).
pub fn v2_mode(ch: usize, pwm: bool) -> [u8; 4] {
    let soft = 0x10u8 << ch;
    [
        REPORT,
        0x10,
        FAN_MODE,
        if pwm { soft | (1 << ch) } else { soft },
    ]
}

/// SL V2 / AL V2 speed: `E0 2x 00 percent`; 0 isn't accepted, so 1.
pub fn v2_speed(ch: usize, duty: f32) -> [u8; 4] {
    [REPORT, SET_SPEED + ch as u8, 0, percent(duty).max(1)]
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Infinity,
    V2,
}

struct Model {
    pid: u16,
    name: &'static str,
    kind: Kind,
}

static MODELS: [Model; 4] = [
    Model {
        pid: 0xA102,
        name: "Lian Li Uni Fan Controller SL Infinity",
        kind: Kind::Infinity,
    },
    Model {
        pid: 0xA103,
        name: "Lian Li Uni Fan Controller SL V2",
        kind: Kind::V2,
    },
    Model {
        pid: 0xA105,
        name: "Lian Li Uni Fan Controller SL V2",
        kind: Kind::V2,
    },
    Model {
        pid: 0xA104,
        name: "Lian Li Uni Fan Controller AL V2",
        kind: Kind::V2,
    },
];

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|m| (ENE, m.pid, m.name)).collect()
}

struct Hub {
    dev: Port,
    usb: Option<Shared>,
    id: String,
    name: &'static str,
    kind: Kind,
    duty: [Option<u8>; CHANNELS],
    software: [bool; CHANNELS],
    rpms: [Option<u32>; CHANNELS],
    read_at: Option<Instant>,
}

impl Hub {
    fn feature(&self, data: &[u8], len: usize) -> Result<(), String> {
        self.dev
            .send_feature_report(&crate::hid::padded(data, len))
            .map_err(|e| e.to_string())
    }
    fn output(&self, data: &[u8], len: usize) -> Result<(), String> {
        self.dev
            .write(&crate::hid::padded(data, len))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn set_mode(&mut self, ch: usize, pwm: bool) -> Result<(), String> {
        match self.kind {
            Kind::Infinity => {
                let mut flush = vec![0u8; inf::RPM_LEN];
                flush[0] = REPORT;
                let _ = self.dev.get_input_report(&mut flush);
                std::thread::sleep(Duration::from_millis(inf::MODE_PAUSE_MS));
                self.feature(&inf_mode(ch, pwm), inf::FEATURE_LEN)?;
            }
            Kind::V2 => {
                self.output(&v2_mode(ch, pwm), v2::MODE_LEN)?;
                std::thread::sleep(Duration::from_millis(v2::MODE_PAUSE_MS));
                self.output(&[REPORT, APPLY], v2::SPEED_LEN)?;
            }
        }
        self.software[ch] = !pwm;
        Ok(())
    }
    fn refresh_infinity(&mut self) {
        if self
            .read_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.read_at = Some(Instant::now());
        self.rpms = [None; CHANNELS];
        if self.feature(&inf::RPM_REQUEST, inf::FEATURE_LEN).is_err() {
            return;
        }
        let mut b = vec![0u8; inf::RPM_LEN];
        b[0] = REPORT;
        if let Ok(n) = self.dev.get_input_report(&mut b) {
            b.truncate(n);
            self.rpms = inf_rpms(&b);
        }
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
        (1..=CHANNELS)
            .map(|i| {
                let mut c = Channel::fan(format!("Channel {i}"));
                c.min_duty = match self.kind {
                    Kind::Infinity => inf::MIN_PERCENT as f32 / 100.0,
                    Kind::V2 => 0.01,
                };
                c
            })
            .collect()
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        match self.kind {
            Kind::Infinity => {
                self.refresh_infinity();
                self.rpms[ch]
            }
            Kind::V2 => {
                let usb = self.usb.as_ref()?.lock().ok()?;
                let b = usb.ctrl_in(v2::RPM_READ[ch], v2::RPM_LEN).ok()?;
                (b.len() >= 2).then(|| u16::from_le_bytes([b[0], b[1]]) as u32)
            }
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        if !self.software[ch] {
            self.set_mode(ch, false)?;
            self.duty[ch] = None;
        }
        let p = percent(duty);
        if self.duty[ch] == Some(p) {
            return Ok(());
        }
        match self.kind {
            Kind::Infinity => {
                std::thread::sleep(Duration::from_millis(inf::MODE_PAUSE_MS));
                self.feature(&inf_speed(ch, duty), inf::FEATURE_LEN)?;
            }
            Kind::V2 => self.output(&v2_speed(ch, duty), v2::SPEED_LEN)?,
        }
        self.duty[ch] = Some(p);
        Ok(())
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.duty[ch] = None;
        if self.software[ch] {
            self.set_mode(ch, true)?;
        }
        Ok(())
    }
}

/// One hub per attached controller, on the collection FoxDen's lighting also uses.
pub fn fans(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut picked: HashMap<String, (Info, &Model)> = HashMap::new();
    for raw in foxden_hid::device_list(api).iter() {
        let info = Info::from_hidapi(raw);
        let Some(m) = MODELS.iter().find(|m| info.vid == ENE && info.pid == m.pid) else {
            continue;
        };
        if info.interface != INTERFACE {
            continue;
        }
        let better = picked
            .get(&info.key())
            .is_none_or(|(old, _)| old.usage_page != USAGE_PAGE && info.usage_page == USAGE_PAGE);
        if better {
            picked.insert(info.key(), (info, m));
        }
    }
    let mut out = Vec::new();
    for (info, m) in picked.into_values() {
        let Ok(dev) = info.port(api) else {
            continue;
        };
        let usb = (m.kind == Kind::V2)
            .then(|| usb::pair(ENE, m.pid, &info.serial))
            .flatten()
            .filter(|s| {
                s.lock()
                    .is_ok_and(|u| u.ctrl_in(v2::RPM_READ[0], v2::RPM_LEN).is_ok())
            });
        out.push(backend_on(
            &info,
            Hub {
                dev,
                usb,
                id: info.device_id(FAMILY).replacen("hid:", "fan:", 1),
                name: m.name,
                kind: m.kind,
                duty: [None; CHANNELS],
                software: [false; CHANNELS],
                rpms: [None; CHANNELS],
                read_at: None,
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // LianLi_SL_Infinity.js: setFanMode / setFanPercent / PollFans.
    #[test]
    fn infinity_packets() {
        assert_eq!(inf_mode(0, false), [0xE0, 0x10, 0x62, 0x10]);
        assert_eq!(inf_mode(2, true), [0xE0, 0x10, 0x62, 0x44]);
        assert_eq!(inf_speed(3, 0.05), [0xE0, 0x23, 0x00, 20]);
        assert_eq!(inf_speed(1, 0.5), [0xE0, 0x21, 0x00, 50]);
        let r = inf_rpms(&[0xE0, 0x04, 0xB0, 0, 0, 0x01, 0x00]);
        assert_eq!(r[0], Some(1200));
        assert_eq!(r[1], Some(0));
        assert_eq!(r[2], Some(256));
        assert_eq!(r[3], None);
    }

    // Lian_Li_Uni_Fans_SL_V2.js: softwareDict, moboDict, setFanPercent.
    #[test]
    fn v2_packets() {
        let soft: Vec<u8> = (0..4).map(|c| v2_mode(c, false)[3]).collect();
        let mobo: Vec<u8> = (0..4).map(|c| v2_mode(c, true)[3]).collect();
        assert_eq!(soft, [0x10, 0x20, 0x40, 0x80]);
        assert_eq!(mobo, [0x11, 0x22, 0x44, 0x88]);
        assert_eq!(v2_speed(2, 0.0), [0xE0, 0x22, 0x00, 1]);
        assert_eq!(v2_speed(0, 1.0), [0xE0, 0x20, 0x00, 100]);
    }
}
