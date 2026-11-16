//! NZXT Kraken X42, X52, X62 and X72 pump, fan and liquid temperature.

use super::util::{Every, be16, hub_id, latest, open, scan, spun};
use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Io, read_until};
use foxrgb_fan::FanBackend;
use hidapi::HidApi;

const VID: u16 = 0x1E71;
const PID: u16 = 0x170E;
const NAME: &str = "NZXT Kraken X2";
const LEN: usize = 64;
const SET: [u8; 2] = [0x02, 0x4D];
const PUMP_BASE: u8 = 0xC0;
const FAN_BASE: u8 = 0x80;
/// Firmware 3 and later only take duty curves.
const CURVE_FIRMWARE: u8 = 3;
const CRITICAL: u8 = 60;
const FAN_DEFAULT: [(u8, u8); 4] = [(20, 25), (30, 50), (50, 90), (60, 100)];
const PUMP_DEFAULT: [(u8, u8); 4] = [(20, 50), (30, 60), (40, 90), (50, 100)];
const SAFE: u8 = 100;

/// The curve's temperatures: every degree to 49, every other to 58, then 60.
pub fn curve_temps() -> Vec<u8> {
    (20..50)
        .chain((50..60).step_by(2))
        .chain([CRITICAL])
        .collect()
}

/// Duty at `t` on a sorted curve, flat outside it.
pub fn interpolate(curve: &[(u8, u8)], t: u8) -> u8 {
    let (first, last) = (curve[0], curve[curve.len() - 1]);
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = curve
        .iter()
        .position(|p| p.0 >= t)
        .unwrap_or(curve.len() - 1);
    let (a, b) = (curve[i - 1], curve[i]);
    let f = f32::from(t - a.0) / f32::from(b.0 - a.0);
    (f32::from(a.1) + f * (f32::from(b.1) - f32::from(a.1))).round() as u8
}

/// One write per curve point; the last point is always full speed.
pub fn curve_packets(base: u8, curve: &[(u8, u8)]) -> Vec<Vec<u8>> {
    curve_temps()
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let duty = if t >= CRITICAL {
                100
            } else {
                interpolate(curve, t)
            };
            vec![SET[0], SET[1], base + i as u8, t, duty.min(100)]
        })
        .collect()
}

/// Older firmware: one fixed duty.
pub fn fixed_packet(base: u8, pct: u8) -> Vec<u8> {
    vec![SET[0], SET[1], base & 0x70, 0, pct.min(100)]
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    pub liquid: f32,
    pub fan: u32,
    pub pump: u32,
    pub firmware: u8,
}

pub fn status(r: &[u8]) -> Option<Status> {
    if r.len() < 15 {
        return None;
    }
    Some(Status {
        liquid: f32::from(r[1]) + f32::from(r[2]) / 10.0,
        fan: be16(r, 3),
        pump: be16(r, 5),
        firmware: r[11],
    })
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    status: Option<Status>,
    seen: bool,
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        let want = |r: &[u8]| r.len() >= 15;
        let r = latest(self.io.as_mut(), LEN, want)
            .or_else(|| read_until(self.io.as_mut(), LEN, 1000, want).ok());
        if let Some(s) = r.as_deref().and_then(status) {
            self.status = Some(s);
        }
    }
    fn curves(&self) -> bool {
        self.status.is_some_and(|s| s.firmware >= CURVE_FIRMWARE)
    }
    fn send(&mut self, base: u8, pct: u8, default: Option<&[(u8, u8)]>) -> Result<(), String> {
        if !self.curves() {
            return self.io.write(&fixed_packet(base, pct), LEN);
        }
        let flat = [(0, pct), (CRITICAL - 1, pct)];
        for p in curve_packets(base, default.unwrap_or(&flat)) {
            self.io.write(&p, LEN)?;
        }
        Ok(())
    }
}

fn base(ch: usize) -> u8 {
    if ch == 0 { PUMP_BASE } else { FAN_BASE }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        NAME.into()
    }
    fn channels(&self) -> Vec<Channel> {
        vec![Channel::pump("Pump"), Channel::fan("Fan")]
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Liquid".into()]
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        let s = self.status?;
        match ch {
            0 => Some(s.pump),
            1 => spun(&mut self.seen, s.fan),
            _ => None,
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        self.refresh();
        self.send(base(ch), percent(duty), None)
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.refresh();
        let default: &[(u8, u8)] = if ch == 0 { &PUMP_DEFAULT } else { &FAN_DEFAULT };
        self.send(base(ch), SAFE, Some(default))
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        self.refresh();
        self.status.map(|s| s.liquid).filter(|t| *t > 0.0)
    }
}

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    vec![(VID, PID, NAME)]
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| i.vid == VID && i.pid == PID)
        .into_iter()
        .filter_map(|info| {
            let mut hub = Hub {
                io: open(api, &info).ok()?,
                id: hub_id(&info, "nzxt-x2"),
                status: None,
                seen: false,
                poll: Every::ms(1000),
            };
            hub.refresh();
            Some(backend_on(&info, hub))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::util::mock::Rec;
    use super::*;

    fn hub(rec: &Rec, firmware: u8) -> Hub {
        let mut h = Hub {
            io: Box::new(rec.clone()),
            id: String::new(),
            status: Some(Status {
                firmware,
                ..Status::default()
            }),
            seen: false,
            poll: Every::ms(60_000),
        };
        h.poll.due();
        h
    }

    #[test]
    fn status_report() {
        let mut r = vec![0u8; 64];
        r[1] = 31;
        r[2] = 4;
        r[3] = 0x03;
        r[4] = 0x84;
        r[5] = 0x0A;
        r[6] = 0x8C;
        r[11] = 6;
        let s = status(&r).unwrap();
        assert_eq!((s.fan, s.pump, s.firmware), (900, 2700, 6));
        assert!((s.liquid - 31.4).abs() < 1e-4);
        assert_eq!(status(&r[..10]), None);
    }

    #[test]
    fn curve_points() {
        let t = curve_temps();
        assert_eq!(t.len(), 36);
        assert_eq!((t[0], t[29], t[30], t[34], t[35]), (20, 49, 50, 58, 60));
        assert_eq!(interpolate(&FAN_DEFAULT, 10), 25);
        assert_eq!(interpolate(&FAN_DEFAULT, 40), 70);
        assert_eq!(interpolate(&PUMP_DEFAULT, 35), 75);
        assert_eq!(interpolate(&PUMP_DEFAULT, 55), 100);
    }

    #[test]
    fn fixed_duty_is_a_flat_curve_then_full_at_60() {
        let rec = Rec::default();
        let mut h = hub(&rec, 6);
        h.set_duty(1, 0.4).unwrap();
        let sent = rec.sent.lock().unwrap();
        assert_eq!(sent.len(), 36);
        assert_eq!(sent[0], crate::hid::padded(&[0x02, 0x4D, 0x80, 20, 40], 64));
        assert_eq!(&sent[34][..5], &[0x02, 0x4D, 0x80 + 34, 58, 40]);
        assert_eq!(&sent[35][..5], &[0x02, 0x4D, 0x80 + 35, 60, 100]);
    }

    #[test]
    fn release_restores_the_default_curves() {
        let rec = Rec::default();
        let mut h = hub(&rec, 6);
        h.release(0).unwrap();
        h.release(1).unwrap();
        let sent = rec.sent.lock().unwrap();
        assert_eq!(&sent[0][..5], &[0x02, 0x4D, 0xC0, 20, 50]);
        assert_eq!(&sent[10][..5], &[0x02, 0x4D, 0xC0 + 10, 30, 60]);
        assert_eq!(&sent[30][..5], &[0x02, 0x4D, 0xC0 + 30, 50, 100]);
        assert_eq!(&sent[36][..5], &[0x02, 0x4D, 0x80, 20, 25]);
        assert_eq!(&sent[66][..5], &[0x02, 0x4D, 0x80 + 30, 50, 90]);
    }

    #[test]
    fn old_firmware_takes_one_fixed_duty() {
        let rec = Rec::default();
        let mut h = hub(&rec, 2);
        h.set_duty(0, 0.2).unwrap();
        h.set_duty(1, 0.3).unwrap();
        h.release(1).unwrap();
        let sent = rec.sent.lock().unwrap();
        let heads: Vec<&[u8]> = sent.iter().map(|p| &p[..5]).collect();
        assert_eq!(
            heads,
            [
                &[0x02, 0x4D, 0x40, 0, 20][..],
                &[0x02, 0x4D, 0x00, 0, 30],
                &[0x02, 0x4D, 0x00, 0, 100],
            ]
        );
    }
}
