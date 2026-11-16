//! NZXT Smart Device V2 and RGB & Fan Controller fans (protocol learned from SignalRGB's
//! Nzxt/NZXT_Smart_Device_V2_Controller.js).

use super::util::{Every, hub_id, latest, le16, open, scan};
use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Io, read_until};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Info;
use hidapi::HidApi;

const VID: u16 = 0x1E71;
const LEN: usize = 64;
const FANS: usize = 3;
const MAX_FANS: usize = 5;
const STATUS: [u8; 2] = [0x67, 0x02];
const H1_STATUS: [u8; 2] = [0x75, 0x02];
const H1_RPM_AT: usize = 23;
const H1_STRIDE: usize = 5;
const HUB_START: [u8; 2] = [0x66, 0x01];
const CONTROL_HUB: u16 = 0x2022;
const H1: u16 = 0x2015;
const MODE_AT: usize = 16;
const RPM_AT: usize = 24;
const SET: [u8; 2] = [0x62, 0x01];
const RATE_RESET: [u8; 2] = [0x60, 0x03];
/// The controller ticks three times a second; a status report every 3 ticks is one a second.
const TICKS: u8 = 3;
const SAFE: u8 = 100;

const MODELS: [(u16, &str); 14] = [
    (0x2006, "NZXT Smart Device V2"),
    (0x200D, "NZXT Smart Device V2 Case Controller"),
    (0x200F, "NZXT Smart Device V2 Case Controller"),
    (0x2009, "NZXT RGB & Fan Controller"),
    (0x200E, "NZXT RGB & Fan Controller"),
    (0x2010, "NZXT RGB & Fan Controller"),
    (0x2011, "NZXT RGB & Fan Controller"),
    (0x2012, "NZXT RGB & Fan Controller"),
    (0x2019, "NZXT RGB & Fan Controller"),
    (0x201F, "NZXT RGB & Fan Controller"),
    (0x2020, "NZXT RGB & Fan Controller"),
    (0x2021, "NZXT RGB & Fan Controller"),
    (CONTROL_HUB, "NZXT Control Hub"),
    (H1, "NZXT H1 V2"),
];

/// Fan headers per model.
fn fans_of(pid: u16) -> usize {
    match pid {
        CONTROL_HUB => MAX_FANS,
        H1 => 2,
        _ => FANS,
    }
}

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|(p, n)| (VID, *p, *n)).collect()
}

pub fn rate(ticks: u8) -> Vec<u8> {
    vec![0x60, 0x02, 0x01, 0xE8, ticks, 0x01, 0xE8, ticks]
}

/// `62 01`, the fan as a bit, then one duty byte per header.
pub fn set(fans: usize, fan: usize, pct: u8) -> Vec<u8> {
    let mut p = vec![SET[0], SET[1], 1 << fan];
    p.resize(3 + fans, 0);
    p[3 + fan] = pct;
    p
}

/// Per fan: rpm, or `None` when the port reports nothing connected.
pub fn status(r: &[u8], fans: usize) -> Vec<Option<u32>> {
    (0..fans)
        .map(|i| (r.get(MODE_AT + i).copied().unwrap_or(0) != 0).then(|| le16(r, RPM_AT + 2 * i)))
        .collect()
}

/// H1 V2 `75 02`: each fan's rpm five bytes after the last.
pub fn h1_status(r: &[u8], fans: usize) -> Vec<Option<u32>> {
    (0..fans)
        .map(|i| Some(le16(r, H1_RPM_AT + H1_STRIDE * i)))
        .collect()
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: &'static str,
    pid: u16,
    rpm: Vec<Option<u32>>,
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        let (h1, fans) = (self.pid == H1, self.rpm.len());
        let head = if h1 { H1_STATUS } else { STATUS };
        let want = |r: &[u8]| r.len() > H1_RPM_AT + H1_STRIDE * fans && r[..2] == head;
        let r = latest(self.io.as_mut(), LEN, want)
            .or_else(|| read_until(self.io.as_mut(), LEN, 1200, want).ok());
        if let Some(r) = r {
            self.rpm = if h1 {
                h1_status(&r, fans)
            } else {
                status(&r, fans)
            };
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
        self.io.write(&set(self.rpm.len(), ch, percent(duty)), LEN)
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.io.write(&set(self.rpm.len(), ch, SAFE), LEN)
    }
}

fn name_of(info: &Info) -> Option<&'static str> {
    let fits = info.vid == VID && matches!(info.interface, -1 | 0);
    MODELS
        .iter()
        .find(|(p, _)| fits && info.pid == *p)
        .map(|m| m.1)
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| name_of(i).is_some())
        .into_iter()
        .filter_map(|info| {
            let mut io = open(api, &info).ok()?;
            if info.pid == CONTROL_HUB {
                io.write(&rate(TICKS), LEN).ok()?;
                io.write(&HUB_START, LEN).ok()?;
            } else {
                io.write(&RATE_RESET, LEN).ok()?;
                io.write(&rate(TICKS), LEN).ok()?;
            }
            Some(backend_on(
                &info,
                Hub {
                    io,
                    id: hub_id(&info, "nzxt-smart2"),
                    name: name_of(&info)?,
                    pid: info.pid,
                    rpm: vec![None; fans_of(info.pid)],
                    poll: Every::ms(1000),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        assert_eq!(set(3, 1, 55), [0x62, 0x01, 0x02, 0, 55, 0]);
        assert_eq!(set(5, 4, 30), [0x62, 0x01, 0x10, 0, 0, 0, 0, 30]);
        assert_eq!(rate(9), [0x60, 0x02, 0x01, 0xE8, 9, 0x01, 0xE8, 9]);
        let mut r = vec![0u8; 64];
        r[..2].copy_from_slice(&STATUS);
        r[16] = 2;
        r[24] = 0xDC;
        r[25] = 0x05;
        r[26] = 0x10;
        assert_eq!(status(&r, 3), [Some(1500), None, None]);
        r[17] = 1;
        r[20] = 1;
        r[32] = 0x20;
        r[33] = 0x03;
        assert_eq!(status(&r, 5), [Some(1500), Some(16), None, None, Some(800)]);
    }

    #[test]
    fn h1_fans_sit_five_bytes_apart() {
        let mut r = vec![0u8; 64];
        r[..2].copy_from_slice(&H1_STATUS);
        r[23] = 0xB0;
        r[24] = 0x04;
        r[28] = 0x58;
        r[29] = 0x02;
        assert_eq!(h1_status(&r, 2), [Some(1200), Some(600)]);
        assert_eq!(
            (fans_of(H1), fans_of(CONTROL_HUB), fans_of(0x2006)),
            (2, 5, 3)
        );
    }
}
