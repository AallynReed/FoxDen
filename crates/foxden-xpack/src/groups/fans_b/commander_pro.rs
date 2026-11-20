//! Corsair Commander Pro and Obsidian 1000D fan and probe side (protocol learned from SignalRGB's
//! Corsair_Lighting_Commander_Pro.js).

use super::util::{Cached, ask, device_key, due, fan_id, u16be};
use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Io, open_io};
use foxden_fan::FanBackend;
use foxden_hid::registry::Info;
use hidapi::HidApi;
use std::time::Instant;

pub const VID: u16 = 0x1B1C;
pub const IDS: &[(u16, &str)] = &[
    (0x0C10, "Corsair Commander Pro"),
    (0x1D00, "Corsair Obsidian Series 1000D Case"),
];
const WRITE_LEN: usize = 65;
const READ_LEN: usize = 17;
pub const FANS: usize = 6;
pub const PROBES: usize = 4;
const GET_PROBES: u8 = 0x10;
const GET_TEMP: u8 = 0x11;
const GET_STATES: u8 = 0x20;
const GET_RPM: u8 = 0x21;
const SET_PCT: u8 = 0x23;
/// Fan states from `0x20`.
const DC: u8 = 0x01;
const PWM: u8 = 0x02;
const PROBE_CONNECTED: u8 = 0x01;
/// The plugin has no hand-back; a released fan is left at this fixed speed.
const RELEASE_PCT: u8 = 75;
const STATE_MS: u64 = 10_000;
const POLL_MS: u64 = 2000;
const RESEND_MS: u64 = 3000;
const REPLY_MS: u64 = 40;

/// `Validate`: interface 0, 2 or none.
pub fn wants(info: &Info) -> bool {
    info.vid == VID && IDS.iter().any(|i| i.0 == info.pid) && matches!(info.interface, -1 | 0 | 2)
}

pub fn request(cmd: u8, arg: Option<u8>) -> Vec<u8> {
    let mut p = vec![0x00, cmd];
    p.extend(arg);
    p
}

pub fn set_pct(fan: usize, pct: u8) -> [u8; 4] {
    [0x00, SET_PCT, fan as u8, pct.min(100)]
}

/// States or probe flags: one byte each after the status byte.
pub fn parse_list(reply: &[u8], n: usize) -> Option<Vec<u8>> {
    Some(reply.get(1..1 + n)?.to_vec())
}

/// RPM and temperature replies: big-endian after the status byte.
pub fn parse_word(reply: &[u8]) -> Option<u16> {
    u16be(reply, 1)
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: String,
    states: Cached<Vec<u8>>,
    probes: Cached<Vec<u8>>,
    rpm: Vec<Cached<u32>>,
    temps: Vec<Cached<f32>>,
    duty: [Option<u8>; FANS],
    sent: [Option<(u8, Instant)>; FANS],
}

impl Hub {
    fn exchange(io: &mut dyn Io, packet: &[u8]) -> Option<Vec<u8>> {
        ask(io, packet, WRITE_LEN, READ_LEN, REPLY_MS, |_| true).ok()
    }

    fn send(&mut self, fan: usize, pct: u8) -> Result<(), String> {
        let fresh = self.sent[fan].is_some_and(|(p, at)| p == pct && !due(Some(at), RESEND_MS));
        if !fresh {
            self.io.write(&set_pct(fan, pct), WRITE_LEN)?;
            self.sent[fan] = Some((pct, Instant::now()));
        }
        Ok(())
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn channels(&self) -> Vec<Channel> {
        (1..=FANS)
            .map(|i| Channel::fan(format!("Fan {i}")))
            .collect()
    }
    fn sensors(&self) -> Vec<String> {
        (1..=PROBES)
            .map(|i| format!("Temperature Probe {i}"))
            .collect()
    }
    fn rpm(&mut self, channel: usize) -> Option<u32> {
        if channel >= FANS {
            return None;
        }
        let io = self.io.as_mut();
        let states = self
            .states
            .get(|| parse_list(&Hub::exchange(io, &request(GET_STATES, None))?, FANS))?;
        if !matches!(states.get(channel), Some(&(DC | PWM))) {
            return None;
        }
        let io = self.io.as_mut();
        let rpm = self.rpm[channel].get(|| {
            let r = Hub::exchange(io, &request(GET_RPM, Some(channel as u8)))?;
            parse_word(&r).map(u32::from)
        });
        if let Some(pct) = self.duty[channel] {
            self.send(channel, pct).ok();
        }
        rpm
    }
    fn set_duty(&mut self, channel: usize, duty: f32) -> Result<(), String> {
        if channel >= FANS {
            return Err("no such channel".into());
        }
        let pct = percent(duty);
        self.duty[channel] = Some(pct);
        self.send(channel, pct)
    }
    fn release(&mut self, channel: usize) -> Result<(), String> {
        if channel >= FANS {
            return Ok(());
        }
        self.duty[channel] = None;
        self.sent[channel] = None;
        self.send(channel, RELEASE_PCT)
    }
    fn temperature(&mut self, sensor: usize) -> Option<f32> {
        if sensor >= PROBES {
            return None;
        }
        let io = self.io.as_mut();
        let flags = self
            .probes
            .get(|| parse_list(&Hub::exchange(io, &request(GET_PROBES, None))?, PROBES))?;
        if flags.get(sensor) != Some(&PROBE_CONNECTED) {
            return None;
        }
        let io = self.io.as_mut();
        self.temps[sensor].get(|| {
            let packet = request(GET_TEMP, Some(sensor as u8));
            let mut raw = parse_word(&Hub::exchange(io, &packet)?)?;
            if raw == 0 {
                raw = parse_word(&io.read(READ_LEN, REPLY_MS as i32).ok()?)?;
            }
            (raw != 0).then_some(f32::from(raw) / 100.0)
        })
    }
}

pub fn hub(api: &HidApi, info: &Info) -> Option<Box<dyn FanBackend>> {
    let name = IDS.iter().find(|i| i.0 == info.pid)?.1;
    let io = open_io(api, info).ok()?;
    Some(backend_on(
        info,
        Hub {
            io,
            id: fan_id(info, "corsair-commander-pro"),
            name: name.into(),
            states: Cached::new(STATE_MS),
            probes: Cached::new(STATE_MS),
            rpm: (0..FANS).map(|_| Cached::new(POLL_MS)).collect(),
            temps: (0..PROBES).map(|_| Cached::new(POLL_MS)).collect(),
            duty: [None; FANS],
            sent: [None; FANS],
        },
    ))
}

pub fn key(info: &Info) -> String {
    device_key(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        assert_eq!(request(GET_STATES, None), vec![0x00, 0x20]);
        assert_eq!(request(GET_RPM, Some(3)), vec![0x00, 0x21, 0x03]);
        assert_eq!(request(GET_TEMP, Some(1)), vec![0x00, 0x11, 0x01]);
        assert_eq!(set_pct(5, 60), [0x00, 0x23, 0x05, 60]);
    }

    #[test]
    fn replies() {
        let states = [0x00, 0x02, 0x00, 0x01, 0x03, 0x00, 0x02, 0xFF];
        assert_eq!(parse_list(&states, FANS), Some(vec![2, 0, 1, 3, 0, 2]));
        assert_eq!(parse_word(&[0x00, 0x04, 0xB0]), Some(1200));
        assert_eq!(
            parse_word(&[0x00, 0x0A, 0x28]).map(|t| t as f32 / 100.0),
            Some(26.0)
        );
    }
}
