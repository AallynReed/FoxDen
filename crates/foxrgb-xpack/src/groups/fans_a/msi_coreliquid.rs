//! MSI MPG Coreliquid K360 radiator fans, water block fan and pump.

use super::util::{Every, hub_id, le16, open, scan, spun};
use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Io, read_until};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Info;
use hidapi::HidApi;

const VID: u16 = 0x0DB0;
const PID: u16 = 0xB130;
const NAME: &str = "MSI MPG Coreliquid K360";
const REPORT: u8 = 0xD0;
const LEN: usize = 64;
const STATUS: u8 = 0x31;
const GET_DUTY: u8 = 0x32;
const GET_TEMPS: u8 = 0x33;
/// The temperature table answers with the duty table's id.
const TEMPS_REPLY: u8 = 0x32;
const SET_DUTY: u8 = 0x40;
const SET_TEMPS: u8 = 0x41;
const CUSTOM: u8 = 3;
const BALANCE: u8 = 1;
const CHANNELS: usize = 5;
const PUMP: usize = 4;
const RECORD: usize = 8;
const REPLY_MS: u64 = 300;
/// The extra mouse-like collection: page 1 with a usage other than 0.
const EXTRA_PAGE: u16 = 0x0001;

/// A channel's mode and its seven curve points.
pub type Record = [u8; RECORD];

/// Five records from a `32` or `33` reply.
pub fn records(r: &[u8], reply: u8) -> Option<[Record; CHANNELS]> {
    if r.get(1) != Some(&reply) || r.len() < 2 + CHANNELS * RECORD {
        return None;
    }
    Some(std::array::from_fn(|i| {
        let at = 2 + i * RECORD;
        r[at..at + RECORD].try_into().unwrap_or([0; RECORD])
    }))
}

/// `D0 40` duties or `D0 41` temperatures, one record per channel.
pub fn table(cmd: u8, recs: &[Record; CHANNELS]) -> Vec<u8> {
    let mut p = vec![REPORT, cmd];
    for r in recs {
        p.extend_from_slice(r);
    }
    p
}

/// Rpm of fans 1-3, the water block fan and the pump.
pub fn rpms(r: &[u8]) -> Option<[u32; CHANNELS]> {
    (r.get(1) == Some(&STATUS)).then(|| std::array::from_fn(|i| le16(r, 2 + 2 * i)))
}

fn preset(mode: u8) -> [Record; CHANNELS] {
    let mut r = [0; RECORD];
    r[0] = mode;
    [r; CHANNELS]
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    duty: [Record; CHANNELS],
    temps: [Record; CHANNELS],
    saved: Option<([Record; CHANNELS], [Record; CHANNELS])>,
    held: [bool; CHANNELS],
    rpm: Option<[u32; CHANNELS]>,
    seen: [bool; CHANNELS],
    poll: Every,
}

impl Hub {
    fn ask(&mut self, cmd: u8, reply: u8) -> Option<Vec<u8>> {
        self.io.write(&[REPORT, cmd], LEN).ok()?;
        read_until(self.io.as_mut(), LEN, REPLY_MS, |r| {
            r.get(1) == Some(&reply)
        })
        .ok()
    }
    fn read_tables(&mut self) {
        let duty = self
            .ask(GET_DUTY, GET_DUTY)
            .and_then(|r| records(&r, GET_DUTY));
        let temps = self
            .ask(GET_TEMPS, TEMPS_REPLY)
            .and_then(|r| records(&r, TEMPS_REPLY));
        if let (Some(d), Some(t)) = (duty, temps) {
            self.duty = d;
            self.temps = t;
            self.saved = Some((d, t));
        }
    }
    fn push(&mut self) -> Result<(), String> {
        self.io.write(&table(SET_DUTY, &self.duty), LEN)?;
        self.io.write(&table(SET_TEMPS, &self.temps), LEN)
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
        vec![
            Channel::fan("Fan 1"),
            Channel::fan("Fan 2"),
            Channel::fan("Fan 3"),
            Channel::fan("Water Block Fan"),
            Channel::pump("Pump"),
        ]
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        if self.poll.due()
            && let Some(r) = self.ask(STATUS, STATUS)
        {
            self.rpm = rpms(&r);
        }
        let rpm = self.rpm?.get(ch).copied()?;
        if ch == PUMP {
            Some(rpm)
        } else {
            spun(self.seen.get_mut(ch)?, rpm)
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        if ch >= CHANNELS {
            return Err("no such channel".into());
        }
        let mut d = [percent(duty); RECORD];
        d[0] = CUSTOM;
        let mut t = [0; RECORD];
        t[0] = CUSTOM;
        self.duty[ch] = d;
        self.temps[ch] = t;
        self.held[ch] = true;
        self.push()
    }
    /// Puts back what the cooler ran before FoxRGB took the channel.
    fn release(&mut self, ch: usize) -> Result<(), String> {
        if ch >= CHANNELS || !self.held[ch] {
            return Ok(());
        }
        let (d, t) = self.saved.unwrap_or((preset(BALANCE), preset(BALANCE)));
        self.duty[ch] = d[ch];
        self.temps[ch] = t[ch];
        self.held[ch] = false;
        self.push()
    }
}

fn wants(info: &Info) -> bool {
    info.vid == VID && info.pid == PID && !(info.usage_page == EXTRA_PAGE && info.usage != 0)
}

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    vec![(VID, PID, NAME)]
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, wants)
        .into_iter()
        .filter_map(|info| {
            let mut hub = Hub {
                io: open(api, &info).ok()?,
                id: hub_id(&info, "msi-coreliquid"),
                duty: preset(BALANCE),
                temps: preset(BALANCE),
                saved: None,
                held: [false; CHANNELS],
                rpm: None,
                seen: [false; CHANNELS],
                poll: Every::ms(1500),
            };
            hub.read_tables();
            Some(backend_on(&info, hub))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::util::mock::Rec;
    use super::*;

    fn reply(cmd: u8, recs: &[Record; CHANNELS]) -> Vec<u8> {
        let mut r = vec![REPORT, cmd];
        for x in recs {
            r.extend_from_slice(x);
        }
        r.resize(LEN, 0);
        r
    }

    #[test]
    fn status_and_tables() {
        let mut r = vec![0u8; LEN];
        r[..2].copy_from_slice(&[REPORT, STATUS]);
        r[2] = 0xE8;
        r[3] = 0x03;
        r[10] = 0xB8;
        r[11] = 0x0B;
        assert_eq!(rpms(&r), Some([1000, 0, 0, 0, 3000]));
        let recs = preset(2);
        assert_eq!(records(&reply(GET_DUTY, &recs), GET_DUTY), Some(recs));
        assert_eq!(records(&reply(GET_DUTY, &recs), STATUS), None);
        let t = table(SET_DUTY, &recs);
        assert_eq!(t.len(), 2 + CHANNELS * RECORD);
        assert_eq!(&t[..3], &[0xD0, 0x40, 2]);
        assert_eq!(t[2 + 4 * RECORD], 2);
    }

    #[test]
    fn duty_writes_and_restore_on_release() {
        let rec = Rec::default();
        let mut before = preset(BALANCE);
        before[0] = [CUSTOM, 10, 20, 30, 40, 50, 60, 70];
        let temps = preset(BALANCE);
        *rec.replies.lock().unwrap() = vec![reply(GET_DUTY, &before), reply(TEMPS_REPLY, &temps)];
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            duty: preset(BALANCE),
            temps: preset(BALANCE),
            saved: None,
            held: [false; CHANNELS],
            rpm: None,
            seen: [false; CHANNELS],
            poll: Every::ms(60_000),
        };
        hub.read_tables();
        rec.sent.lock().unwrap().clear();

        hub.set_duty(0, 0.35).unwrap();
        hub.set_duty(PUMP, 0.8).unwrap();
        let mut duty = before;
        duty[0] = [CUSTOM, 35, 35, 35, 35, 35, 35, 35];
        duty[PUMP] = [CUSTOM, 80, 80, 80, 80, 80, 80, 80];
        let mut t = temps;
        t[0] = [CUSTOM, 0, 0, 0, 0, 0, 0, 0];
        t[PUMP] = t[0];
        {
            let sent = rec.sent.lock().unwrap();
            assert_eq!(sent[2], crate::hid::padded(&table(SET_DUTY, &duty), LEN));
            assert_eq!(sent[3], crate::hid::padded(&table(SET_TEMPS, &t), LEN));
        }

        hub.release(0).unwrap();
        hub.release(PUMP).unwrap();
        let sent = rec.sent.lock().unwrap();
        assert_eq!(sent[6], crate::hid::padded(&table(SET_DUTY, &before), LEN));
        assert_eq!(sent[7], crate::hid::padded(&table(SET_TEMPS, &temps), LEN));
        assert_eq!(sent.len(), 8);
    }
}
