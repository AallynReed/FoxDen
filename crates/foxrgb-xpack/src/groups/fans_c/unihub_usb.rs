//! Lian Li Uni Fan controllers over vendor control transfers (protocol learned from SignalRGB's Lian_Li_Uni_Fans.js, Lian_Li_Uni_Fans_SL_1.2.js and Lian_Li_Uni_Fans_AL_1.1.js).

use super::usb::{self, Shared, Usb, UsbDriver, UsbProto, pause};
use crate::fan::{Channel, FanHub, backend, percent};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_fan::FanBackend;

pub const ENE: u16 = 0x0CF2;
pub const CHANNELS: usize = 4;
/// Four fans of 16 LEDs on each channel.
pub const CHANNEL_LEDS: usize = 64;
const RECT: [f32; 4] = [0.06, 0.24, 0.52, 0.24];
const FAMILY: &str = "lianli-unifan";
/// Duty 1.0 is this many rpm on every control-transfer hub.
const FULL_RPM: f32 = 1900.0;

/// Uni Fan controller 0cf2:7750: its own register map.
pub mod v1 {
    pub const COMMAND: u16 = 0xE021;
    pub const COMMIT: u16 = 0xE02F;
    pub const ACTION: [u16; 4] = [0xE300, 0xE3C0, 0xE480, 0xE540];
    pub const CH_COMMIT: [u16; 4] = [0xE02F, 0xE03F, 0xE04F, 0xE05F];
    /// Mode register; speed, direction and brightness follow at +1, +2 and +8.
    pub const MODE: [u16; 4] = [0xE021, 0xE031, 0xE041, 0xE051];
    pub const FAN_ACTION: [u16; 4] = [0xE8A0, 0xE8A2, 0xE8A4, 0xE8A6];
    pub const FAN_COMMIT: [u16; 4] = [0xE890, 0xE891, 0xE892, 0xE893];
    pub const FAN_READ: [u16; 4] = [0xE800, 0xE802, 0xE804, 0xE806];
    pub const FAN_PWM_COMMIT: [u16; 4] = [0xE818, 0xE81A, 0xE81C, 0xE81E];
    pub const FAN_CMD: u8 = 0x31;
    pub const MANUAL: u8 = 0xF0;
    pub const PWM: u8 = 0xFF;
    pub const START: u8 = 0x34;
    pub const FAN_COUNT: u8 = 0x32;
    pub const PASSTHROUGH: u8 = 0x30;
    /// Channels whose speed was never set get this percent when manual mode starts (BurstFans).
    pub const BURST: f32 = 0.75;
}

/// SL 1.0/1.2 (0cf2:a100, a106) and AL 1.0/1.1 (0cf2:a101): 16-byte commands at 0xE020.
pub mod v2 {
    pub const COMMAND: u16 = 0xE020;
    pub const ACTION: [u16; 4] = [0xE300, 0xE3C0, 0xE480, 0xE540];
    pub const CH_COMMIT: [u16; 4] = [0xE020, 0xE030, 0xE040, 0xE050];
    pub const FAN_ACTION: [u16; 4] = [0xD8A0, 0xD8A2, 0xD8A4, 0xD8A6];
    pub const FAN_COMMIT: [u16; 4] = [0xD890, 0xD891, 0xD892, 0xD893];
    pub const FAN_READ: [u16; 4] = [0xD800, 0xD802, 0xD804, 0xD806];
    /// The controller refuses less than this many rpm.
    pub const MIN_RPM: u16 = 800;
    pub const PWM_SYNC: u8 = 0x31;
    pub const START: u8 = 0x34;
    pub const FAN_COUNT: u8 = 0x32;
    pub const PASSTHROUGH: u8 = 0x30;
    pub const APPLY: u8 = 0x01;
    /// Colour data and commits go out at this length.
    pub const DATA_LEN: usize = 192;
}

/// A 16-byte v2 command: `[0, op, a, b, 0.., 1]`.
pub fn cmd16(op: u8, a: u8, b: u8) -> [u8; 16] {
    let mut p = [0u8; 16];
    p[1] = op;
    p[2] = a;
    p[3] = b;
    p[15] = 0x01;
    p
}

/// Duty 0..=1 as the controller's rpm target, little endian.
pub fn rpm_bytes(duty: f32, floor: u16) -> [u8; 2] {
    let rpm = ((FULL_RPM * percent(duty) as f32 / 100.0).round() as u16).max(floor);
    rpm.to_le_bytes()
}

/// One channel's colours, R B G per LED, zero padded to 64 LEDs.
pub fn rbg(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; CHANNEL_LEDS * 3];
    for (i, c) in colors.iter().take(CHANNEL_LEDS).enumerate() {
        b[i * 3..i * 3 + 3].copy_from_slice(&[c.0, c.2, c.1]);
    }
    b
}

/// The v2 per-channel motherboard PWM hand-back packet.
pub fn v2_pwm(ch: usize) -> [u8; 16] {
    cmd16(v2::PWM_SYNC, (0x10u8 << ch) | 0x0F, 0)
}

#[derive(Clone, Copy, PartialEq)]
enum Map {
    V1,
    V2,
}

pub struct Model {
    pub pid: u16,
    pub name: &'static str,
    map: Map,
    pub lighting: bool,
}

/// Controllers this module drives. Lighting only where FoxRGB has none.
pub static MODELS: [Model; 4] = [
    Model {
        pid: 0x7750,
        name: "Lian Li Uni Fan Controller",
        map: Map::V1,
        lighting: true,
    },
    Model {
        pid: 0xA106,
        name: "Lian Li Uni Fan Controller SL 1.2",
        map: Map::V2,
        lighting: true,
    },
    Model {
        pid: 0xA100,
        name: "Lian Li Uni Fan Controller SL 1.2",
        map: Map::V2,
        lighting: false,
    },
    Model {
        pid: 0xA101,
        name: "Lian Li Uni Fan Controller AL 1.1",
        map: Map::V2,
        lighting: false,
    },
];

fn fan_read(map: Map, ch: usize) -> u16 {
    match map {
        Map::V1 => v1::FAN_READ[ch],
        Map::V2 => v2::FAN_READ[ch],
    }
}

/// Opens every attached controller of a model that answers an rpm read (a HID-bound hub on
/// Windows doesn't until WinUSB is installed).
fn opened(m: &Model) -> Vec<Shared> {
    usb::find(ENE, m.pid)
        .iter()
        .filter_map(|d| usb::open(d, None).ok())
        .filter(|s| {
            s.lock()
                .is_ok_and(|u| u.ctrl_in(fan_read(m.map, 0), 2).is_ok())
        })
        .collect()
}

struct V1Lights;

impl V1Lights {
    fn send(usb: &Usb, index: u16, data: &[u8]) -> Result<(), String> {
        usb.ctrl_out(index, data)
    }
    fn commit(usb: &Usb) -> Result<(), String> {
        usb.ctrl_out(v1::COMMIT, &[0x01])
    }
}

impl UsbProto for V1Lights {
    fn start(&mut self, usb: &Usb) -> Result<(), String> {
        Self::send(usb, v1::COMMAND, &[v1::PASSTHROUGH, 0])?;
        Self::commit(usb)?;
        for m in v1::MODE {
            Self::send(usb, m, &[1])?;
            Self::send(usb, m + 1, &[1])?;
            Self::send(usb, m + 2, &[0])?;
            Self::send(usb, m + 8, &[0])?;
            Self::send(usb, m + 0x0E, &[1])?;
        }
        Ok(())
    }
    fn frame(&mut self, usb: &Usb, colors: &[Color]) -> Result<(), String> {
        Self::send(usb, v1::COMMAND, &[v1::START])?;
        Self::commit(usb)?;
        for ch in 0..CHANNELS as u8 {
            Self::send(usb, v1::COMMAND, &[v1::FAN_COUNT, 0x10 * ch + 3])?;
            Self::commit(usb)?;
        }
        for (ch, c) in colors.chunks(CHANNEL_LEDS).enumerate().take(CHANNELS) {
            Self::send(usb, v1::ACTION[ch], &rbg(c))?;
            pause(1);
            Self::send(usb, v1::CH_COMMIT[ch], &[1])?;
        }
        Ok(())
    }
    fn stop(&mut self, usb: &Usb) {
        let _ = self.frame(usb, &[Color(0, 0, 0); CHANNELS * CHANNEL_LEDS]);
    }
}

struct V2Lights;

impl V2Lights {
    fn commit_packet() -> Vec<u8> {
        let mut p = vec![0u8; v2::DATA_LEN];
        p[..16].copy_from_slice(&cmd16(v2::APPLY, 0xFF, 0));
        p
    }
}

impl UsbProto for V2Lights {
    fn start(&mut self, usb: &Usb) -> Result<(), String> {
        for ch in 0..CHANNELS as u8 {
            usb.ctrl_out(v2::COMMAND, &cmd16(v2::FAN_COUNT, 0x10 * ch + 3, 0))?;
            pause(1);
        }
        usb.ctrl_out(v2::COMMAND, &cmd16(v2::START, 0, 0))?;
        usb.ctrl_out(v2::COMMAND, &cmd16(v2::PASSTHROUGH, 0, 0))?;
        usb.ctrl_out(v2::COMMAND, &cmd16(v2::APPLY, 0x02, 0))
    }
    fn frame(&mut self, usb: &Usb, colors: &[Color]) -> Result<(), String> {
        let commit = Self::commit_packet();
        for (ch, c) in colors.chunks(CHANNEL_LEDS).enumerate().take(CHANNELS) {
            usb.ctrl_out(v2::ACTION[ch], &rbg(c))?;
            pause(5);
            usb.ctrl_out(v2::CH_COMMIT[ch], &commit)?;
            pause(5);
        }
        Ok(())
    }
    fn stop(&mut self, usb: &Usb) {
        let _ = self.frame(usb, &[Color(0, 0, 0); CHANNELS * CHANNEL_LEDS]);
    }
}

fn layout(id: String, name: &str) -> Device {
    Device::grid(
        &id,
        name,
        DeviceKind::Other,
        RECT,
        CHANNEL_LEDS as u32,
        CHANNELS as u32,
    )
}

/// Lighting backends for the raw-USB controllers FoxRGB doesn't light.
pub fn lights() -> Vec<Box<dyn Backend>> {
    let mut out = Vec::new();
    for m in MODELS.iter().filter(|m| m.lighting) {
        for s in opened(m) {
            let Ok(id) = s.lock().map(|u| u.device_id(FAMILY)) else {
                continue;
            };
            let dev = layout(id, m.name);
            out.push(match m.map {
                Map::V1 => UsbDriver::boxed(s, dev, "lian li uni fan", V1Lights),
                Map::V2 => UsbDriver::boxed(s, dev, "lian li uni fan sl 1.2", V2Lights),
            });
        }
    }
    out
}

struct Hub {
    usb: Shared,
    id: String,
    name: &'static str,
    map: Map,
    duty: [Option<u8>; CHANNELS],
    released: [bool; CHANNELS],
    touched: [bool; CHANNELS],
    manual: bool,
}

impl Hub {
    fn write_rpm(&self, usb: &Usb, ch: usize, duty: f32) -> Result<(), String> {
        match self.map {
            Map::V1 => {
                usb.ctrl_out(v1::FAN_ACTION[ch], &rpm_bytes(duty, 0))?;
                usb.ctrl_out(v1::FAN_COMMIT[ch], &[1])
            }
            Map::V2 => {
                usb.ctrl_out(v2::FAN_ACTION[ch], &rpm_bytes(duty, v2::MIN_RPM))?;
                pause(1);
                usb.ctrl_out(v2::FAN_COMMIT[ch], &[1])
            }
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
                if self.map == Map::V2 {
                    c.min_duty = v2::MIN_RPM as f32 / FULL_RPM;
                }
                c
            })
            .collect()
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        let usb = self.usb.lock().ok()?;
        let b = usb.ctrl_in(fan_read(self.map, ch), 2).ok()?;
        (b.len() >= 2).then(|| u16::from_le_bytes([b[0], b[1]]) as u32)
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        let p = percent(duty);
        if self.duty[ch] == Some(p) && !self.released[ch] {
            return Ok(());
        }
        let usb = self.usb.clone();
        let usb = usb.lock().map_err(|e| e.to_string())?;
        if self.map == Map::V1 && !self.manual {
            usb.ctrl_out(v1::COMMAND, &[v1::FAN_CMD, v1::MANUAL])?;
            usb.ctrl_out(v1::COMMIT, &[1])?;
            self.manual = true;
            for other in (0..CHANNELS).filter(|&o| o != ch && self.duty[o].is_none()) {
                self.write_rpm(&usb, other, v1::BURST)?;
            }
        }
        self.write_rpm(&usb, ch, duty)?;
        self.duty[ch] = Some(p);
        self.released[ch] = false;
        self.touched[ch] = true;
        Ok(())
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        let usb = self.usb.clone();
        let usb = usb.lock().map_err(|e| e.to_string())?;
        self.released[ch] = true;
        match self.map {
            Map::V2 => {
                if self.touched[ch] {
                    usb.ctrl_out(v2::COMMAND, &v2_pwm(ch))?;
                    self.touched[ch] = false;
                }
            }
            Map::V1 => {
                if !self.manual || self.released.iter().any(|r| !r) {
                    return Ok(());
                }
                for o in 0..CHANNELS {
                    usb.ctrl_out(v1::FAN_COMMIT[o], &[0, 0])?;
                    usb.ctrl_out(v1::FAN_PWM_COMMIT[o], &[1])?;
                }
                usb.ctrl_out(v1::COMMAND, &[v1::FAN_CMD, v1::PWM])?;
                usb.ctrl_out(v1::COMMIT, &[1])?;
                self.manual = false;
            }
        }
        Ok(())
    }
}

/// One fan hub per attached control-transfer controller.
pub fn fans() -> Vec<Box<dyn FanBackend>> {
    let mut out = Vec::new();
    for m in &MODELS {
        for s in opened(m) {
            let Ok(id) = s.lock().map(|u| u.fan_id(FAMILY)) else {
                continue;
            };
            out.push(backend(Hub {
                usb: s,
                id,
                name: m.name,
                map: m.map,
                duty: [None; CHANNELS],
                released: [false; CHANNELS],
                touched: [false; CHANNELS],
                manual: false,
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lian_Li_Uni_Fans_SL_1.2.js: setFanPercent / setFanRPM.
    #[test]
    fn rpm_targets() {
        assert_eq!(rpm_bytes(1.0, 0), 1900u16.to_le_bytes());
        assert_eq!(rpm_bytes(0.5, 0), 950u16.to_le_bytes());
        assert_eq!(rpm_bytes(0.1, v2::MIN_RPM), 800u16.to_le_bytes());
    }

    // setMoboPassthrough / setFanMode packet shapes.
    #[test]
    fn v2_commands() {
        let p = cmd16(v2::PASSTHROUGH, 0, 0);
        assert_eq!(p[1], 0x30);
        assert_eq!(p[15], 0x01);
        assert_eq!(v2_pwm(0)[2], 0x1F);
        assert_eq!(v2_pwm(3)[2], 0x8F);
        assert_eq!(V2Lights::commit_packet()[..4], [0x00, 0x01, 0xFF, 0x00]);
        assert_eq!(V2Lights::commit_packet().len(), 192);
    }

    // getChannelColors: "Inline", "RBG".
    #[test]
    fn rbg_order() {
        let b = rbg(&[Color(1, 2, 3)]);
        assert_eq!(b.len(), 192);
        assert_eq!(b[..3], [1, 3, 2]);
    }
}
