//! Aqua Computer D5 Next, Octo, Quadro, Farbwerk and Farbwerk 360 fans, pump and temperatures.

use super::util::{Every, be16, hub_id, latest, open, scan, spun};
use crate::fan::{Channel, FanHub, backend_on};
use crate::hid::{Io, read_until};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Info;
use hidapi::HidApi;

const VID: u16 = 0x0C70;
const STATUS: u8 = 0x01;
const CTRL: u8 = 0x03;
const DISCONNECTED: u32 = 0x7FFF;
const RPM_AT: usize = 8;
const MANUAL: u8 = 0;
/// The device wants a pause between reading its settings and writing them back.
const SETTLE_MS: u64 = 200;

pub struct Model {
    pid: u16,
    name: &'static str,
    status_len: usize,
    ctrl_len: usize,
    /// (name, pump, rpm block in the status report, settings block in the control report)
    fans: &'static [(&'static str, bool, usize, usize)],
    temps: &'static [(&'static str, usize)],
}

const PROBES_FARBWERK: &[(&str, usize)] = &[
    ("Sensor 1", 0x2F),
    ("Sensor 2", 0x31),
    ("Sensor 3", 0x33),
    ("Sensor 4", 0x35),
];

pub const MODELS: &[Model] = &[
    Model {
        pid: 0xF00E,
        name: "Aqua Computer D5 Next",
        status_len: 0x9E,
        ctrl_len: 0x329,
        fans: &[("Pump", true, 0x6C, 0x96), ("Fan", false, 0x5F, 0x41)],
        temps: &[("Liquid", 0x57)],
    },
    Model {
        pid: 0xF011,
        name: "Aqua Computer Octo",
        status_len: 0x147,
        ctrl_len: 0x65F,
        fans: &[
            ("Fan 1", false, 0x7D, 0x5A),
            ("Fan 2", false, 0x8A, 0xAF),
            ("Fan 3", false, 0x97, 0x104),
            ("Fan 4", false, 0xA4, 0x159),
            ("Fan 5", false, 0xB1, 0x1AE),
            ("Fan 6", false, 0xBE, 0x203),
            ("Fan 7", false, 0xCB, 0x258),
            ("Fan 8", false, 0xD8, 0x2AD),
        ],
        temps: &[
            ("Sensor 1", 0x3D),
            ("Sensor 2", 0x3F),
            ("Sensor 3", 0x41),
            ("Sensor 4", 0x43),
        ],
    },
    Model {
        pid: 0xF00D,
        name: "Aqua Computer Quadro",
        status_len: 0xDC,
        ctrl_len: 0x3C1,
        fans: &[
            ("Fan 1", false, 0x70, 0x36),
            ("Fan 2", false, 0x7D, 0x8B),
            ("Fan 3", false, 0x8A, 0xE0),
            ("Fan 4", false, 0x97, 0x135),
        ],
        temps: &[
            ("Sensor 1", 0x34),
            ("Sensor 2", 0x36),
            ("Sensor 3", 0x38),
            ("Sensor 4", 0x3A),
        ],
    },
    Model {
        pid: 0xF010,
        name: "Aqua Computer Farbwerk 360",
        status_len: 0xB6,
        ctrl_len: 0,
        fans: &[],
        temps: &[
            ("Sensor 1", 0x32),
            ("Sensor 2", 0x34),
            ("Sensor 3", 0x36),
            ("Sensor 4", 0x38),
        ],
    },
    Model {
        pid: 0xF00A,
        name: "Aqua Computer Farbwerk",
        status_len: 0xB6,
        ctrl_len: 0,
        fans: &[],
        temps: PROBES_FARBWERK,
    },
];

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|m| (VID, m.pid, m.name)).collect()
}

/// CRC-16/USB: reflected 0x8005, start 0xFFFF, inverted at the end.
pub fn crc16_usb(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Seals a control report: the checksum of bytes 1..len-2 goes big-endian in its last two.
pub fn seal(report: &mut [u8]) {
    let n = report.len();
    if n < 4 {
        return;
    }
    let crc = crc16_usb(&report[1..n - 2]);
    report[n - 2..].copy_from_slice(&crc.to_be_bytes());
}

/// A channel in manual mode at `pct` percent, in hundredths, big-endian.
pub fn set_manual(report: &mut [u8], at: usize, pct: u8) {
    if let Some(b) = report.get_mut(at..at + 3) {
        let v = (u16::from(pct.min(100)) * 100).to_be_bytes();
        b.copy_from_slice(&[MANUAL, v[0], v[1]]);
    }
}

/// A temperature in hundredths of a degree, `None` for an empty input.
pub fn temp(r: &[u8], at: usize) -> Option<f32> {
    if r.len() < at + 2 {
        return None;
    }
    let v = be16(r, at);
    (v != DISCONNECTED).then(|| v as f32 / 100.0)
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    model: &'static Model,
    status: Option<Vec<u8>>,
    /// Each channel's settings before FoxRGB first set it.
    saved: Vec<Option<[u8; 3]>>,
    sent: Vec<Option<u8>>,
    seen: Vec<bool>,
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        let len = self.model.status_len;
        let want = |r: &[u8]| r.first() == Some(&STATUS) && r.len() >= len;
        let r = latest(self.io.as_mut(), len, want)
            .or_else(|| read_until(self.io.as_mut(), len, 1500, want).ok());
        if r.is_some() {
            self.status = r;
        }
    }
    fn settings(&mut self) -> Result<Vec<u8>, String> {
        let len = self.model.ctrl_len;
        let r = self.io.get_feature(CTRL, len)?;
        if r.len() != len || r[0] != CTRL {
            return Err("short control report".into());
        }
        Ok(r)
    }
    fn write(&mut self, ch: usize, block: [u8; 3]) -> Result<(), String> {
        let at = self.model.fans.get(ch).ok_or("no such channel")?.3;
        let mut r = self.settings()?;
        if self.saved[ch].is_none() {
            self.saved[ch] = r[at..at + 3].try_into().ok();
        }
        r[at..at + 3].copy_from_slice(&block);
        seal(&mut r);
        self.io.sleep(SETTLE_MS);
        let len = r.len();
        self.io.feature(&r, len)
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
        self.model
            .fans
            .iter()
            .map(|f| {
                if f.1 {
                    Channel::pump(f.0)
                } else {
                    Channel::fan(f.0)
                }
            })
            .collect()
    }
    fn sensors(&self) -> Vec<String> {
        self.model.temps.iter().map(|t| t.0.into()).collect()
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        let f = self.model.fans.get(ch)?;
        let rpm = be16(self.status.as_deref()?, f.2 + RPM_AT);
        if f.1 {
            Some(rpm)
        } else {
            spun(self.seen.get_mut(ch)?, rpm)
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        let pct = crate::fan::percent(duty);
        if self.sent.get(ch) == Some(&Some(pct)) {
            return Ok(());
        }
        let mut block = [0u8; 3];
        set_manual(&mut block, 0, pct);
        self.write(ch, block)?;
        self.sent[ch] = Some(pct);
        Ok(())
    }
    /// Puts the channel's own settings back.
    fn release(&mut self, ch: usize) -> Result<(), String> {
        let Some(block) = self.saved.get(ch).copied().flatten() else {
            return Ok(());
        };
        self.write(ch, block)?;
        self.saved[ch] = None;
        self.sent[ch] = None;
        Ok(())
    }
    fn temperature(&mut self, sensor: usize) -> Option<f32> {
        self.refresh();
        temp(self.status.as_deref()?, self.model.temps.get(sensor)?.1)
    }
}

fn model_of(info: &Info) -> Option<&'static Model> {
    MODELS.iter().find(|m| info.vid == VID && info.pid == m.pid)
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| model_of(i).is_some())
        .into_iter()
        .filter_map(|info| {
            let model = model_of(&info)?;
            let n = model.fans.len();
            Some(backend_on(
                &info,
                Hub {
                    io: open(api, &info).ok()?,
                    id: hub_id(&info, "aquacomputer"),
                    model,
                    status: None,
                    saved: vec![None; n],
                    sent: vec![None; n],
                    seen: vec![false; n],
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

    #[test]
    fn crc_matches_the_usb_check_value() {
        assert_eq!(crc16_usb(b"123456789"), 0xB4C8);
    }

    #[test]
    fn status_rpm_and_temperatures() {
        let quadro = &MODELS[2];
        let mut r = vec![0u8; quadro.status_len];
        r[0] = STATUS;
        r[0x34..0x36].copy_from_slice(&2512u16.to_be_bytes());
        r[0x36..0x38].copy_from_slice(&0x7FFFu16.to_be_bytes());
        r[0x7D + 8..0x7D + 10].copy_from_slice(&980u16.to_be_bytes());
        assert_eq!(temp(&r, 0x34), Some(25.12));
        assert_eq!(temp(&r, 0x36), None);
        let rec = Rec::default();
        *rec.replies.lock().unwrap() = vec![r];
        let mut hub = Hub {
            io: Box::new(rec),
            id: String::new(),
            model: quadro,
            status: None,
            saved: vec![None; 4],
            sent: vec![None; 4],
            seen: vec![false; 4],
            poll: Every::ms(60_000),
        };
        assert_eq!(hub.rpm(1), Some(980));
        assert_eq!(hub.rpm(0), None);
        assert_eq!(hub.temperature(0), Some(25.12));
    }

    #[test]
    fn duty_rewrites_the_control_report_and_release_restores_it() {
        let d5 = &MODELS[0];
        let mut before = vec![0u8; d5.ctrl_len];
        before[0] = CTRL;
        before[0x96..0x99].copy_from_slice(&[2, 0x11, 0x22]);
        before[0x200] = 7;
        seal(&mut before);
        let rec = Rec::default();
        let mut hub = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            model: d5,
            status: None,
            saved: vec![None; 2],
            sent: vec![None; 2],
            seen: vec![false; 2],
            poll: Every::ms(60_000),
        };
        *rec.replies.lock().unwrap() = vec![before.clone()];
        hub.set_duty(0, 0.65).unwrap();
        hub.set_duty(0, 0.65).unwrap();
        let mut want = before.clone();
        want[0x96..0x99].copy_from_slice(&[0, 0x19, 0x64]);
        seal(&mut want);
        assert_eq!(rec.sent.lock().unwrap()[0], want);

        *rec.replies.lock().unwrap() = vec![want];
        hub.release(0).unwrap();
        assert_eq!(rec.sent.lock().unwrap()[1], before);
        hub.release(0).unwrap();
        assert_eq!(rec.sent.lock().unwrap().len(), 2);
    }
}
