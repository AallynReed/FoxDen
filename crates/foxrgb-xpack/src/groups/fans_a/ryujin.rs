//! ASUS ROG Ryujin AIO fans and pumps (protocol learned from SignalRGB's Asus/ASUS_Ryujin_AIO_Controller.js).

use super::util::{Every, hub_id, iface, le16, open, scan, spun};
use crate::fan::{Channel, FanHub, backend_on, byte, percent};
use crate::hid::{Io, drain, read_until};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Info;
use hidapi::HidApi;

const VID: u16 = 0x0B05;
const PAGE: u16 = 0xFF72;
const USAGE: u16 = 0x00A1;
const REPORT: u8 = 0xEC;
const LEN: usize = 65;
const STATUS: u8 = 0x99;
const DUTY: u8 = 0x9A;
const SET_PUMP: u8 = 0x1A;
const HUB_RPM: u8 = 0xA0;
const SET_HUB: u8 = 0x21;
const GEN1_STATUS: u8 = 0xAA;
const GEN1_SET: u8 = 0x2A;
const SAFE: u8 = 100;
const REPLY_MS: u64 = 250;

#[derive(Clone, Copy)]
struct Model {
    pid: u16,
    name: &'static str,
    interface: i32,
    gen1: bool,
    hub: bool,
    lcd: bool,
    ryuo: bool,
}

const MODELS: [Model; 7] = [
    Model {
        pid: 0x18AE,
        name: "ASUS Ryujin I",
        interface: 0,
        gen1: true,
        hub: false,
        lcd: false,
        ryuo: false,
    },
    Model {
        pid: 0x1988,
        name: "ASUS Ryujin II",
        interface: 1,
        gen1: false,
        hub: true,
        lcd: false,
        ryuo: false,
    },
    Model {
        pid: 0x1AA2,
        name: "ASUS Ryujin III",
        interface: 1,
        gen1: false,
        hub: false,
        lcd: true,
        ryuo: false,
    },
    Model {
        pid: 0x1ADA,
        name: "ASUS Ryujin III White",
        interface: 1,
        gen1: false,
        hub: false,
        lcd: true,
        ryuo: false,
    },
    Model {
        pid: 0x1BCB,
        name: "ASUS Ryujin III Extreme",
        interface: 1,
        gen1: false,
        hub: false,
        lcd: true,
        ryuo: false,
    },
    Model {
        pid: 0x1ADE,
        name: "ASUS Ryujin III EVA",
        interface: 1,
        gen1: false,
        hub: false,
        lcd: true,
        ryuo: false,
    },
    Model {
        pid: 0x1887,
        name: "ASUS ROG Ryuo",
        interface: 0,
        gen1: false,
        hub: false,
        lcd: false,
        ryuo: true,
    },
];

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|m| (VID, m.pid, m.name)).collect()
}

/// Replies carry the command with its top bit cleared, after the report id.
pub fn reply_to(cmd: u8) -> [u8; 2] {
    [REPORT, cmd & 0x7F]
}

/// The third generation addresses its cooler as channel 1.
pub fn pump_packet(gen1: bool, lcd: bool, pump: u8, fan: u8) -> Vec<u8> {
    if gen1 {
        vec![REPORT, GEN1_SET, pump, fan]
    } else {
        vec![REPORT, SET_PUMP, u8::from(lcd), pump, fan]
    }
}

/// Ryuo: one duty for its radiator fans.
pub fn ryuo_packet(pct: u8) -> Vec<u8> {
    vec![REPORT, GEN1_SET, pct]
}

pub fn hub_packet(duty: f32) -> Vec<u8> {
    vec![REPORT, SET_HUB, 0x00, 0x00, byte(duty)]
}

#[derive(Default, Debug, PartialEq)]
pub struct Status {
    pub pump: Option<u32>,
    pub fan: u32,
    pub liquid: Option<f32>,
}

/// `EC 19` reply: liquid temperature, pump and radiator fan rpm (LCD models sit a few bytes later).
pub fn status(r: &[u8], lcd: bool) -> Status {
    let (a, b) = if lcd { (2, 3) } else { (0, 0) };
    Status {
        pump: Some(le16(r, 5 + a)),
        fan: le16(r, 7 + b),
        liquid: Some(r[3 + a] as f32 + r[4 + a] as f32 / 10.0),
    }
}

/// `EC 2A` reply of the first generation: radiator fan rpm in hundreds, no pump rpm.
pub fn gen1_status(r: &[u8]) -> Status {
    Status {
        pump: None,
        fan: r.get(4).copied().unwrap_or(0) as u32 * 100,
        liquid: None,
    }
}

/// `EC 20` reply: the four hub fans, the fourth stored first.
pub fn hub_rpms(r: &[u8]) -> [u32; 4] {
    [le16(r, 5), le16(r, 7), le16(r, 9), le16(r, 3)]
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    model: Model,
    pump: u8,
    fan: u8,
    status: Status,
    hub_rpm: [u32; 4],
    seen: [bool; 3],
    poll: Every,
}

impl Hub {
    fn ask(&mut self, cmd: u8) -> Option<Vec<u8>> {
        drain(self.io.as_mut(), LEN);
        self.io.write(&[REPORT, cmd], LEN).ok()?;
        let want = reply_to(cmd);
        read_until(self.io.as_mut(), LEN, REPLY_MS, |r| {
            r.len() > 12 && r[..2] == want
        })
        .ok()
    }
    fn refresh(&mut self) {
        if self.model.ryuo || !self.poll.due() {
            return;
        }
        if self.model.gen1 {
            if let Some(r) = self.ask(GEN1_STATUS) {
                self.status = gen1_status(&r);
            }
            return;
        }
        if let Some(r) = self.ask(STATUS) {
            self.status = status(&r, self.model.lcd);
        }
        if self.model.hub
            && let Some(r) = self.ask(HUB_RPM)
        {
            self.hub_rpm = hub_rpms(&r);
        }
    }
    /// Seeds the shared pump packet with what the cooler runs now.
    fn seed(&mut self) {
        if !self.model.gen1
            && !self.model.ryuo
            && let Some(r) = self.ask(DUTY)
            && r[4] > 0
        {
            self.pump = r[4].max(SAFE / 2);
            self.fan = r[5];
        }
    }
    fn send(&mut self, ch: usize, pct: u8) -> Result<(), String> {
        if self.model.ryuo {
            return self.io.write(&ryuo_packet(pct), LEN);
        }
        match ch {
            0 => self.pump = pct,
            1 => self.fan = pct,
            _ => return self.io.write(&hub_packet(pct as f32 / 100.0), LEN),
        }
        self.io.write(
            &pump_packet(self.model.gen1, self.model.lcd, self.pump, self.fan),
            LEN,
        )
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.model.name.into()
    }
    fn channels(&self) -> Vec<Channel> {
        if self.model.ryuo {
            return vec![Channel::fan("Fans")];
        }
        let mut c = vec![Channel::pump("Pump"), Channel::fan("Pump Fan")];
        if self.model.hub {
            c.push(Channel::fan("Fans"));
        }
        c
    }
    fn sensors(&self) -> Vec<String> {
        if self.model.gen1 || self.model.ryuo {
            Vec::new()
        } else {
            vec!["Liquid".into()]
        }
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        if self.model.ryuo {
            return None;
        }
        match ch {
            0 => self.status.pump,
            1 => spun(&mut self.seen[1], self.status.fan),
            2 => {
                let max = self.hub_rpm.iter().copied().max().unwrap_or(0);
                spun(&mut self.seen[2], max)
            }
            _ => None,
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        self.send(ch, percent(duty))
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.send(ch, SAFE)
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        self.refresh();
        self.status.liquid.filter(|t| *t > 0.0)
    }
}

fn model_of(info: &Info) -> Option<Model> {
    MODELS.iter().copied().find(|m| {
        info.vid == VID
            && info.pid == m.pid
            && (m.ryuo || iface(info, m.interface))
            && info.usage_page == PAGE
            && info.usage == USAGE
    })
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| model_of(i).is_some())
        .into_iter()
        .filter_map(|info| {
            let model = model_of(&info)?;
            let mut hub = Hub {
                io: open(api, &info).ok()?,
                id: hub_id(&info, "asus-ryujin"),
                model,
                pump: SAFE,
                fan: SAFE,
                status: Status::default(),
                hub_rpm: [0; 4],
                seen: [false; 3],
                poll: Every::ms(1500),
            };
            hub.seed();
            Some(backend_on(&info, hub))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        assert_eq!(reply_to(STATUS), [0xEC, 0x19]);
        assert_eq!(reply_to(GEN1_STATUS), [0xEC, 0x2A]);
        assert_eq!(pump_packet(false, false, 80, 40), [0xEC, 0x1A, 0, 80, 40]);
        assert_eq!(pump_packet(false, true, 80, 40), [0xEC, 0x1A, 1, 80, 40]);
        assert_eq!(pump_packet(true, false, 80, 40), [0xEC, 0x2A, 80, 40]);
        assert_eq!(ryuo_packet(45), [0xEC, 0x2A, 45]);
        assert_eq!(hub_packet(1.0), [0xEC, 0x21, 0, 0, 255]);
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 65];
        r[..2].copy_from_slice(&[0xEC, 0x19]);
        r[3] = 31;
        r[4] = 5;
        r[5] = 0x0A;
        r[6] = 0x05;
        r[7] = 0x20;
        r[8] = 0x03;
        let s = status(&r, false);
        assert_eq!(
            (s.pump, s.fan, s.liquid),
            (Some(0x050A), 0x0320, Some(31.5))
        );
        let mut h = vec![0u8; 65];
        h[3] = 1;
        h[5] = 2;
        assert_eq!(hub_rpms(&h), [2, 0, 0, 1]);
    }

    #[test]
    fn fan_write_keeps_pump() {
        let rec = super::super::util::mock::Rec::default();
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            model: MODELS[1],
            pump: SAFE,
            fan: SAFE,
            status: Status::default(),
            hub_rpm: [0; 4],
            seen: [false; 3],
            poll: Every::ms(1500),
        };
        hub.set_duty(1, 0.3).unwrap();
        hub.set_duty(2, 0.5).unwrap();
        let sent = rec.sent.lock().unwrap();
        assert_eq!(&sent[0][..5], &[0xEC, 0x1A, 0, 100, 30]);
        assert_eq!(&sent[1][..5], &[0xEC, 0x21, 0, 0, 128]);
    }
}
