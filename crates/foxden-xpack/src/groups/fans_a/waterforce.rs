//! Gigabyte AORUS Waterforce X AIOs.

use super::util::{Every, cells, hub_id, iface, open, scan, spun};
use crate::fan::{Channel, FanHub, backend_on};
use crate::hid::{Driver, Io, Proto, drain, open_io, read_until};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_fan::FanBackend;
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const REPORT: u8 = 0x99;
const OUT: usize = 6144;
const IN: usize = 256;
const SPEEDS: u8 = 0xDA;
const MODES: u8 = 0xDD;
const SET_MODE: u8 = 0xE5;
const CURVE: u8 = 0xE6;
const COLOR: u8 = 0xCD;
const FAN: u8 = 1;
const PUMP: u8 = 2;
const CUSTOM: u8 = 1;
const TEMPS: [u8; 4] = [0, 30, 50, 65];
const MAX_RPM: f32 = 2800.0;
const PUMP_MIN_RPM: u16 = 1600;
/// Curves left behind on release: rising with liquid temperature (ours, not the plugin's).
const FAN_RAMP: [u16; 4] = [840, 1400, 2100, 2800];
const PUMP_RAMP: [u16; 4] = [1600, 2000, 2400, 2800];

const MODELS: [(u16, u16, &str); 2] = [
    (0x1044, 0x7A4D, "Gigabyte AORUS Waterforce X I"),
    (0x0414, 0x7A5E, "Gigabyte AORUS Waterforce X II"),
];

pub const IDS: &[(u16, u16, &str)] = &MODELS;

/// A fixed-rpm curve: four (temperature, rpm high, rpm low) points.
pub fn curve(target: u8, rpm: [u16; 4]) -> Vec<u8> {
    let mut p = vec![REPORT, CURVE, 0x00, target];
    for (t, r) in TEMPS.iter().zip(rpm) {
        p.extend_from_slice(&[*t, (r >> 8) as u8, r as u8]);
    }
    p
}

/// Rpm a duty asks for; the pump never goes under its floor.
pub fn rpm_for(target: u8, duty: f32) -> u16 {
    let r = (MAX_RPM * duty.clamp(0.0, 1.0)).round() as u16;
    if target == PUMP {
        r.max(PUMP_MIN_RPM)
    } else {
        r
    }
}

pub fn mode(target: u8, mode: u8) -> Vec<u8> {
    vec![REPORT, SET_MODE, target, mode, 0x00]
}

/// `99 DA` reply: fan and pump rpm, 24-bit little endian.
pub fn speeds(r: &[u8]) -> [u32; 2] {
    let at = |i: usize| {
        (0..3).fold(0u32, |v, k| {
            v | (r.get(i + k).copied().unwrap_or(0) as u32) << (8 * k)
        })
    };
    [at(2), at(5)]
}

fn ask(io: &mut dyn Io, cmd: u8) -> Option<Vec<u8>> {
    drain(io, IN);
    io.write(&[REPORT, cmd], OUT).ok()?;
    read_until(io, IN, 300, |r| r.len() > 7 && r[..2] == [REPORT, cmd]).ok()
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: &'static str,
    /// Fan and pump modes found at open, restored on release.
    modes: Option<[u8; 2]>,
    custom: [bool; 2],
    rpm: [u32; 2],
    seen: [bool; 2],
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if self.poll.due()
            && let Some(r) = ask(self.io.as_mut(), SPEEDS)
        {
            self.rpm = speeds(&r);
        }
    }
    fn target(ch: usize) -> u8 {
        if ch == 0 { PUMP } else { FAN }
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
        let mut pump = Channel::pump("Pump");
        pump.min_duty = PUMP_MIN_RPM as f32 / MAX_RPM;
        vec![pump, Channel::fan("Fan")]
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        let rpm = if ch == 0 { self.rpm[1] } else { self.rpm[0] };
        spun(self.seen.get_mut(ch)?, rpm)
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        let t = Self::target(ch);
        if !self.custom[ch] {
            self.io.write(&mode(t, CUSTOM), OUT)?;
            self.io.sleep(10);
            self.custom[ch] = true;
        }
        self.io.write(&curve(t, [rpm_for(t, duty); 4]), OUT)
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        let t = Self::target(ch);
        let ramp = if t == PUMP { PUMP_RAMP } else { FAN_RAMP };
        self.io.write(&curve(t, ramp), OUT)?;
        if let Some(m) = self.modes {
            let was = if t == FAN { m[0] } else { m[1] };
            self.io.write(&mode(t, was), OUT)?;
        }
        self.custom[ch] = false;
        Ok(())
    }
}

fn name_of(info: &Info) -> Option<&'static str> {
    MODELS
        .iter()
        .find(|(v, p, _)| info.vid == *v && info.pid == *p && iface(info, 0))
        .map(|m| m.2)
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| name_of(i).is_some())
        .into_iter()
        .filter_map(|info| {
            let mut io = open(api, &info).ok()?;
            let modes = ask(io.as_mut(), MODES).map(|r| [r[2], r[3]]);
            Some(backend_on(
                &info,
                Hub {
                    io,
                    id: hub_id(&info, "aorus-waterforce"),
                    name: name_of(&info)?,
                    modes,
                    custom: [false; 2],
                    rpm: [0; 2],
                    seen: [false; 2],
                    poll: Every::ms(1500),
                },
            ))
        })
        .collect()
}

/// Waterforce X I: one colour for the whole pump cap.
struct Lights;

impl Proto for Lights {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or(Color(0, 0, 0));
        io.write(&[REPORT, COLOR, c.0, c.1, c.2], OUT)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&[REPORT, COLOR, 0, 0, 0], OUT);
    }
}

fn open_lights(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(
        &info.device_id("aorus-waterforce"),
        d.name,
        DeviceKind::Other,
        [0.45, 0.10, 0.06, 0.06],
        [1, 1],
        &[[0, 0]],
    );
    Ok(Driver::new(open_io(api, info)?, dev, "aorus-waterforce", Lights).boxed())
}

pub static DETECTORS: [Detector; 2] = [
    crate::hid::on_i(MODELS[0].2, 0x1044, 0x7A4D, 0, open_lights),
    crate::hid::on_i(MODELS[0].2, 0x1044, 0x7A4D, -1, open_lights),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        let p = curve(PUMP, [rpm_for(PUMP, 0.2); 4]);
        assert_eq!(&p[..10], &[0x99, 0xE6, 0, 2, 0, 0x06, 0x40, 30, 0x06, 0x40]);
        assert_eq!(p.len(), 16);
        assert_eq!(rpm_for(FAN, 0.5), 1400);
        assert_eq!(rpm_for(FAN, 0.0), 0);
        assert_eq!(mode(FAN, 1), [0x99, 0xE5, 1, 1, 0]);
        let r = [0x99, 0xDA, 0xE8, 0x03, 0, 0xD0, 0x07, 0];
        assert_eq!(speeds(&r), [1000, 2000]);
    }
}
