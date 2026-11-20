//! NZXT Smart Device (V1) and Grid+ V3 fans.

use super::util::{Every, be16, hub_id, open, scan};
use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Io, read_until};
use foxden_fan::FanBackend;
use foxden_hid::registry::Info;
use hidapi::HidApi;

const VID: u16 = 0x1E71;
const LEN: usize = 65;
const REPORT_LEN: usize = 21;
const DETECT: [u8; 2] = [0x01, 0x5C];
const START: [u8; 2] = [0x01, 0x5D];
const SET: [u8; 2] = [0x02, 0x4D];
const SAFE: u8 = 100;

const MODELS: [(u16, &str, usize); 2] = [
    (0x1714, "NZXT Smart Device V1", 3),
    (0x1711, "NZXT Grid+ V3", 6),
];

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|m| (VID, m.0, m.1)).collect()
}

pub fn set(fan: usize, pct: u8) -> Vec<u8> {
    vec![SET[0], SET[1], fan as u8, 0, pct.min(100)]
}

/// One fan's report: (index, rpm), rpm `None` while nothing is plugged in.
pub fn fan_report(r: &[u8]) -> Option<(usize, Option<u32>)> {
    let b = *r.get(15)?;
    if r.len() < REPORT_LEN - 1 {
        return None;
    }
    Some((usize::from(b >> 4), (b & 0x03 != 0).then(|| be16(r, 3))))
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: &'static str,
    rpm: Vec<Option<u32>>,
    poll: Every,
}

impl Hub {
    fn take(&mut self, r: &[u8]) {
        if let Some((i, rpm)) = fan_report(r)
            && let Some(slot) = self.rpm.get_mut(i)
        {
            *slot = rpm;
        }
    }
    /// Each report carries one fan, so a pass reads what is queued, or waits for one.
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        let mut got = false;
        while let Ok(r) = self.io.read(REPORT_LEN, 0) {
            if r.is_empty() {
                break;
            }
            got = true;
            self.take(&r);
        }
        if !got && let Ok(r) = read_until(self.io.as_mut(), REPORT_LEN, 500, |_| true) {
            self.take(&r);
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
        (1..=self.rpm.len())
            .map(|i| Channel::fan(format!("Fan {i}")))
            .collect()
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        *self.rpm.get(ch)?
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        self.io.write(&set(ch, percent(duty)), LEN)
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.io.write(&set(ch, SAFE), LEN)
    }
}

fn model_of(info: &Info) -> Option<(&'static str, usize)> {
    MODELS
        .iter()
        .find(|m| info.vid == VID && info.pid == m.0)
        .map(|m| (m.1, m.2))
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| model_of(i).is_some())
        .into_iter()
        .filter_map(|info| {
            let (name, fans) = model_of(&info)?;
            let mut io = open(api, &info).ok()?;
            io.write(&DETECT, LEN).ok()?;
            io.write(&START, LEN).ok()?;
            Some(backend_on(
                &info,
                Hub {
                    io,
                    id: hub_id(&info, "nzxt-grid"),
                    name,
                    rpm: vec![None; fans],
                    poll: Every::ms(1000),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::util::mock::Rec;
    use super::*;

    fn report(fan: u8, state: u8, rpm: u16) -> Vec<u8> {
        let mut r = vec![0u8; REPORT_LEN];
        r[3..5].copy_from_slice(&rpm.to_be_bytes());
        r[15] = fan << 4 | state;
        r
    }

    #[test]
    fn packets_and_reports() {
        assert_eq!(set(2, 45), [0x02, 0x4D, 2, 0, 45]);
        assert_eq!(fan_report(&report(1, 2, 1200)), Some((1, Some(1200))));
        assert_eq!(fan_report(&report(4, 0, 0)), Some((4, None)));
        assert_eq!(fan_report(&[0u8; 8]), None);
    }

    #[test]
    fn reports_fill_their_fans_and_release_sets_full_speed() {
        let rec = Rec::default();
        *rec.replies.lock().unwrap() = vec![report(0, 1, 900), report(2, 2, 1500)];
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            name: "G",
            rpm: vec![None; 6],
            poll: Every::ms(60_000),
        };
        assert_eq!(hub.rpm(0), Some(900));
        assert_eq!(hub.rpm(2), Some(1500));
        assert_eq!(hub.rpm(1), None);
        hub.set_duty(5, 0.3).unwrap();
        hub.release(5).unwrap();
        let sent = rec.sent.lock().unwrap();
        assert_eq!(sent[0], crate::hid::padded(&[0x02, 0x4D, 5, 0, 30], LEN));
        assert_eq!(sent[1], crate::hid::padded(&[0x02, 0x4D, 5, 0, 100], LEN));
    }
}
