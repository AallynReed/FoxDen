//! Razer PWM fan controller (protocol learned from SignalRGB's Razer/Razer_PWM_Controller.js).

use super::util::{Every, be16, hub_id, iface, open, scan, spun};
use crate::fan::{Channel, FanHub, backend, percent};
use crate::hid::Io;
use foxrgb_fan::FanBackend;
use hidapi::HidApi;

const VID: u16 = 0x1532;
const PID: u16 = 0x0F3C;
const NAME: &str = "Razer PWM Controller";
const FANS: usize = 8;
const LEN: usize = 91;
const TRANSACTION: u8 = 0x1F;
const CLASS: u8 = 0x0D;
const SET_MODE: u8 = 0x02;
const SET_SPEED: u8 = 0x0D;
const GET_RPM: u8 = 0x81;
const MANUAL: u8 = 0x04;
const FIRST_CHANNEL: u8 = 0x05;
const CRC_AT: usize = 89;
const RPM_AT: usize = 13;
const SAFE: u8 = 100;

pub const IDS: &[(u16, u16, &str)] = &[(VID, PID, NAME)];

/// A 91-byte feature report: report id 0, transaction, data size, class, command, args, XOR crc.
pub fn packet(size: u8, command: u8, args: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; LEN];
    b[2] = TRANSACTION;
    b[6] = size;
    b[7] = CLASS;
    b[8] = command;
    b[9..9 + args.len()].copy_from_slice(args);
    b[CRC_AT] = b[3..CRC_AT].iter().fold(0, |a, x| a ^ x);
    b
}

pub fn manual(ch: u8) -> Vec<u8> {
    packet(3, SET_MODE, &[0x01, FIRST_CHANNEL + ch, MANUAL])
}

pub fn speed(ch: u8, pct: u8) -> Vec<u8> {
    packet(3, SET_SPEED, &[0x01, FIRST_CHANNEL + ch, pct])
}

pub fn rpm_query(ch: u8) -> Vec<u8> {
    packet(6, GET_RPM, &[0x01, FIRST_CHANNEL + ch])
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    manual: [bool; FANS],
    rpm: [u32; FANS],
    seen: [bool; FANS],
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        for ch in 0..FANS {
            if self.io.feature(&rpm_query(ch as u8), LEN).is_err() {
                continue;
            }
            self.io.sleep(10);
            if let Ok(r) = self.io.get_feature(0, LEN)
                && r.len() > RPM_AT + 1
            {
                self.rpm[ch] = be16(&r, RPM_AT);
            }
        }
    }
    fn send(&mut self, ch: usize, pct: u8) -> Result<(), String> {
        if !self.manual[ch] {
            self.io.feature(&manual(ch as u8), LEN)?;
            self.io.sleep(10);
            self.manual[ch] = true;
        }
        self.io.feature(&speed(ch as u8, pct), LEN)?;
        self.io.sleep(10);
        Ok(())
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        NAME.into()
    }
    fn channels(&self) -> Vec<Channel> {
        (1..=FANS)
            .map(|i| Channel::fan(format!("Fan {i}")))
            .collect()
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        spun(self.seen.get_mut(ch)?, self.rpm[ch])
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        self.send(ch, percent(duty))
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.send(ch, SAFE)
    }
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    let want = |i: &foxrgb_hid::registry::Info| i.vid == VID && i.pid == PID && iface(i, 0);
    scan(api, seen, want)
        .into_iter()
        .filter_map(|info| {
            Some(backend(Hub {
                io: open(api, &info).ok()?,
                id: hub_id(&info, "razer-pwm"),
                manual: [false; FANS],
                rpm: [0; FANS],
                seen: [false; FANS],
                poll: Every::ms(2000),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        let p = speed(2, 60);
        assert_eq!(p.len(), 91);
        assert_eq!(&p[..12], &[0, 0, 0x1F, 0, 0, 0, 3, 0x0D, 0x0D, 1, 7, 60]);
        assert_eq!(p[89], 3 ^ 0x0D ^ 0x0D ^ 1 ^ 7 ^ 60);
        assert_eq!(&manual(0)[6..12], &[3, 0x0D, 0x02, 1, 5, 4]);
        assert_eq!(&rpm_query(7)[6..11], &[6, 0x0D, 0x81, 1, 12]);
    }

    #[test]
    fn manual_mode_once_and_rpm_reply() {
        let rec = super::super::util::mock::Rec::default();
        let mut reply = vec![0u8; LEN];
        reply[13] = 0x04;
        reply[14] = 0xB0;
        rec.replies.lock().unwrap().push(reply);
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            manual: [false; FANS],
            rpm: [0; FANS],
            seen: [false; FANS],
            poll: Every::ms(2000),
        };
        hub.set_duty(0, 0.5).unwrap();
        hub.set_duty(0, 0.6).unwrap();
        assert_eq!(hub.rpm(0), Some(1200));
        let sent = rec.sent.lock().unwrap();
        assert_eq!(sent[0], manual(0));
        assert_eq!(sent[1], speed(0, 50));
        assert_eq!(sent[2], speed(0, 60));
    }
}
