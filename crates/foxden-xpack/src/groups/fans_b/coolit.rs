//! Corsair H110i GT pump, fans and liquid temperature.

use super::util::{Cached, ask, device_key, fan_id, u16le};
use crate::fan::{Channel, FanHub, backend_on, byte};
use crate::hid::{Io, open_io};
use foxden_fan::FanBackend;
use foxden_hid::registry::Info;
use hidapi::HidApi;

pub const VID: u16 = 0x1B1C;
pub const IDS: &[(u16, &str)] = &[(0x0C04, "Corsair H110i GT")];
const WRITE_LEN: usize = 65;
const READ_LEN: usize = 64;
const REPLY_MS: u64 = 500;
const POLL_MS: u64 = 1500;

const WRITE_1: u8 = 0x06;
const WRITE_2: u8 = 0x08;
const READ_2: u8 = 0x09;
const TEMP: u8 = 0x0E;
const SELECT: u8 = 0x10;
const MODE: u8 = 0x12;
const PWM: u8 = 0x13;
const FIXED_RPM: u8 = 0x14;
const RPM: u8 = 0x16;
const MODE_PWM: u8 = 0x02;
const PUMP_SLOT: u8 = 2;
const PUMP_QUIET: u16 = 2350;
const PUMP_EXTREME: u16 = 2950;
/// Pump duty from which the faster of its two speeds is used.
const EXTREME_FROM: f32 = 0.75;
const SAFE: f32 = 1.0;

/// Steps 2..=31, then wraps to 1.
pub fn next_seq(seq: &mut u8) -> u8 {
    *seq = *seq % 31 + 1;
    *seq
}

/// One buffer: report id 0, the byte count, then each `seq op cmd params` command.
pub fn buffer(seq: &mut u8, cmds: &[(u8, u8, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for &(op, cmd, params) in cmds {
        body.extend([next_seq(seq), op, cmd]);
        body.extend_from_slice(params);
    }
    let mut b = vec![0x00, body.len() as u8];
    b.extend(body);
    b
}

/// Liquid temperature and the rpm of fan 1, fan 2 and the pump.
pub fn status_cmds() -> Vec<(u8, u8, &'static [u8])> {
    vec![
        (READ_2, TEMP, &[]),
        (WRITE_1, SELECT, &[0]),
        (READ_2, RPM, &[]),
        (WRITE_1, SELECT, &[1]),
        (READ_2, RPM, &[]),
        (WRITE_1, SELECT, &[PUMP_SLOT]),
        (READ_2, RPM, &[]),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub liquid: f32,
    pub rpm: [u32; 3],
}

pub fn status(r: &[u8]) -> Option<Status> {
    let rpm = |at| u16le(r, at).map(u32::from);
    Some(Status {
        liquid: f32::from(*r.get(3)?) + f32::from(*r.get(2)?) / 255.0,
        rpm: [rpm(8)?, rpm(14)?, rpm(20)?],
    })
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: &'static str,
    seq: u8,
    status: Cached<Status>,
    sent: [Option<u8>; 3],
}

impl Hub {
    fn send(&mut self, cmds: &[(u8, u8, &[u8])]) -> Result<Vec<u8>, String> {
        let b = buffer(&mut self.seq, cmds);
        ask(self.io.as_mut(), &b, WRITE_LEN, READ_LEN, REPLY_MS, |_| {
            true
        })
    }
    fn fan(&mut self, slot: u8, duty: f32) -> Result<(), String> {
        self.send(&[(WRITE_1, SELECT, &[slot])])?;
        self.send(&[(WRITE_1, MODE, &[MODE_PWM])])?;
        self.send(&[(WRITE_1, PWM, &[byte(duty)])]).map(|_| ())
    }
    fn pump(&mut self, duty: f32) -> Result<(), String> {
        let rpm = if duty >= EXTREME_FROM {
            PUMP_EXTREME
        } else {
            PUMP_QUIET
        };
        let speed = rpm.to_le_bytes();
        self.send(&[
            (WRITE_1, SELECT, &[PUMP_SLOT]),
            (WRITE_2, FIXED_RPM, &speed),
        ])
        .map(|_| ())
    }
    fn set(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        match ch {
            0 => self.pump(duty),
            1 | 2 => self.fan(ch as u8 - 1, duty),
            _ => Err("no such channel".into()),
        }
    }
    fn read(&mut self) -> Option<Status> {
        let (io, seq) = (&mut self.io, &mut self.seq);
        self.status.get(|| {
            let b = buffer(seq, &status_cmds());
            status(&ask(io.as_mut(), &b, WRITE_LEN, READ_LEN, REPLY_MS, |_| true).ok()?)
        })
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
        vec![
            Channel::pump("Pump"),
            Channel::fan("Fan 1"),
            Channel::fan("Fan 2"),
        ]
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Liquid".into()]
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        let s = self.read()?;
        match ch {
            0 => Some(s.rpm[2]),
            1 | 2 => Some(s.rpm[ch - 1]),
            _ => None,
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        let b = byte(duty);
        if self.sent.get(ch) == Some(&Some(b)) {
            return Ok(());
        }
        self.set(ch, duty)?;
        self.sent[ch] = Some(b);
        Ok(())
    }
    /// The pump goes back to its quiet speed, fans to full.
    fn release(&mut self, ch: usize) -> Result<(), String> {
        if let Some(s) = self.sent.get_mut(ch) {
            *s = None;
        }
        self.set(ch, if ch == 0 { 0.0 } else { SAFE })
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        self.read().map(|s| s.liquid).filter(|t| *t > 0.0)
    }
}

pub fn wants(info: &Info) -> bool {
    info.vid == VID && IDS.iter().any(|i| i.0 == info.pid)
}

pub fn hub(api: &HidApi, info: &Info) -> Option<Box<dyn FanBackend>> {
    let name = IDS.iter().find(|i| i.0 == info.pid)?.1;
    Some(backend_on(
        info,
        Hub {
            io: open_io(api, info).ok()?,
            id: fan_id(info, "corsair-coolit"),
            name,
            seq: 1,
            status: Cached::new(POLL_MS),
            sent: [None; 3],
        },
    ))
}

pub fn key(info: &Info) -> String {
    device_key(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records writes and answers each with one empty reply.
    #[derive(Clone, Default)]
    struct Rec {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        owed: Arc<Mutex<bool>>,
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push(crate::hid::padded(data, len));
            *self.owed.lock().unwrap() = true;
            Ok(())
        }
        fn feature(&mut self, _: &[u8], _: usize) -> Result<(), String> {
            Ok(())
        }
        fn get_feature(&mut self, _: u8, _: usize) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
        fn read(&mut self, len: usize, _: i32) -> Result<Vec<u8>, String> {
            let owed = std::mem::take(&mut *self.owed.lock().unwrap());
            Ok(if owed { vec![0; len] } else { Vec::new() })
        }
    }

    #[test]
    fn sequence_runs_2_to_31_then_1() {
        let mut s = 1;
        let seen: Vec<u8> = (0..32).map(|_| next_seq(&mut s)).collect();
        assert_eq!(seen[0], 2);
        assert_eq!(seen[29], 31);
        assert_eq!(seen[30], 1);
        assert_eq!(seen[31], 2);
    }

    #[test]
    fn status_buffer_and_reply() {
        let mut s = 1;
        let b = buffer(&mut s, &status_cmds());
        assert_eq!(
            b,
            [
                0, 24, 2, 0x09, 0x0E, 3, 0x06, 0x10, 0, 4, 0x09, 0x16, 5, 0x06, 0x10, 1, 6, 0x09,
                0x16, 7, 0x06, 0x10, 2, 8, 0x09, 0x16
            ]
        );
        let mut r = vec![0u8; 64];
        r[2] = 128;
        r[3] = 30;
        r[8..10].copy_from_slice(&1100u16.to_le_bytes());
        r[14..16].copy_from_slice(&1200u16.to_le_bytes());
        r[20..22].copy_from_slice(&2350u16.to_le_bytes());
        let st = status(&r).unwrap();
        assert_eq!(st.rpm, [1100, 1200, 2350]);
        assert!((st.liquid - 30.502).abs() < 0.01);
    }

    #[test]
    fn duty_and_release_writes() {
        let rec = Rec::default();
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            name: "H",
            seq: 1,
            status: Cached::new(60_000),
            sent: [None; 3],
        };
        hub.set_duty(2, 0.4).unwrap();
        hub.set_duty(2, 0.4).unwrap();
        hub.set_duty(0, 0.9).unwrap();
        hub.release(2).unwrap();
        hub.release(0).unwrap();
        let sent = rec.sent.lock().unwrap();
        let p = |b: &[u8]| crate::hid::padded(b, WRITE_LEN);
        assert_eq!(sent[0], p(&[0, 4, 2, 0x06, 0x10, 1]));
        assert_eq!(sent[1], p(&[0, 4, 3, 0x06, 0x12, 0x02]));
        assert_eq!(sent[2], p(&[0, 4, 4, 0x06, 0x13, 102]));
        assert_eq!(
            sent[3],
            p(&[0, 9, 5, 0x06, 0x10, 2, 6, 0x08, 0x14, 0x86, 0x0B])
        );
        assert_eq!(sent[6], p(&[0, 4, 9, 0x06, 0x13, 255]));
        assert_eq!(
            sent[7],
            p(&[0, 9, 10, 0x06, 0x10, 2, 11, 0x08, 0x14, 0x2E, 0x09])
        );
    }
}
