//! Arctic fan controller (protocol learned from SignalRGB's Arctic/Arctic_Fan_Controller.js).

use super::util::{Every, hub_id, iface, latest, open, scan, spun};
use crate::fan::{Channel, FanHub, backend, percent};
use crate::hid::{Io, read_until};
use foxrgb_fan::FanBackend;
use hidapi::HidApi;

const VID: u16 = 0x3904;
const PID: u16 = 0xF001;
const NAME: &str = "Arctic Fan Controller";
const FANS: usize = 10;
const REPORT: u8 = 0x01;
const LEN: usize = 32;
const RPM_AT: usize = 11;
/// Duty every channel starts at and goes back to on release (one packet sets all ten).
const SAFE: u8 = 100;

pub const IDS: &[(u16, u16, &str)] = &[(VID, PID, NAME)];

/// Sets all ten channels at once, in percent.
pub fn speeds(duty: &[u8; FANS]) -> Vec<u8> {
    let mut p = vec![REPORT];
    p.extend_from_slice(duty);
    p
}

/// Ten little-endian rpms from a status report.
pub fn rpms(r: &[u8]) -> Option<[u32; FANS]> {
    if r.len() < LEN || r[0] != REPORT {
        return None;
    }
    Some(std::array::from_fn(|i| {
        super::util::le16(r, RPM_AT + 2 * i)
    }))
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    duty: [u8; FANS],
    rpm: [u32; FANS],
    seen: [bool; FANS],
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        let ok = |r: &[u8]| rpms(r).is_some();
        let r = latest(self.io.as_mut(), 64, ok)
            .or_else(|| read_until(self.io.as_mut(), 64, 300, ok).ok());
        if let Some(v) = r.as_deref().and_then(rpms) {
            self.rpm = v;
        }
    }
    fn send(&mut self, ch: usize, pct: u8) -> Result<(), String> {
        self.duty[ch] = pct;
        self.io.write(&speeds(&self.duty), LEN)
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
    let want = |i: &foxrgb_hid::registry::Info| {
        i.vid == VID && i.pid == PID && (i.usage_page == 0xFF00 || iface(i, 0))
    };
    scan(api, seen, want)
        .into_iter()
        .filter_map(|info| {
            let io = open(api, &info).ok()?;
            Some(backend(Hub {
                io,
                id: hub_id(&info, "arctic"),
                duty: [SAFE; FANS],
                rpm: [0; FANS],
                seen: [false; FANS],
                poll: Every::ms(1000),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        let mut d = [SAFE; FANS];
        d[2] = 40;
        assert_eq!(
            speeds(&d),
            [1, 100, 100, 40, 100, 100, 100, 100, 100, 100, 100]
        );
        let mut r = vec![0u8; 32];
        r[0] = 1;
        r[11] = 0x34;
        r[12] = 0x12;
        r[29] = 0x10;
        assert_eq!(rpms(&r).unwrap()[0], 0x1234);
        assert_eq!(rpms(&r).unwrap()[9], 0x10);
        r[0] = 2;
        assert!(rpms(&r).is_none());
    }

    #[test]
    fn other_channels_keep_their_duty() {
        let rec = super::super::util::mock::Rec::default();
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            duty: [SAFE; FANS],
            rpm: [0; FANS],
            seen: [false; FANS],
            poll: Every::ms(1000),
        };
        hub.set_duty(1, 0.3).unwrap();
        hub.set_duty(4, 0.5).unwrap();
        let sent = rec.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(&sent[1][..6], &[1, 100, 30, 100, 100, 50]);
        assert_eq!(sent[1].len(), LEN);
    }
}
