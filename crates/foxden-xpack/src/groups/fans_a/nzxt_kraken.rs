//! NZXT Kraken X3, Z3, 2023 and Elite V2 pump and fan control.

use super::util::{Every, hub_id, iface, latest, le16, open, scan, spun};
use crate::fan::{Channel, FanHub, backend_on, percent};
use crate::hid::{Driver, Io, Proto, drain, open_io, read_until, set_rows, span};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_fan::FanBackend;
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x1E71;
const STATUS_ASK: [u8; 2] = [0x74, 0x01];
const STATUS: u8 = 0x75;
const FIRMWARE_ASK: [u8; 2] = [0x10, 0x02];
const FIRMWARE: [u8; 2] = [0x11, 0x02];
const CURVE: u8 = 0x72;
const PUMP: u8 = 0x01;
const FAN: u8 = 0x02;
/// The curve holds one duty per degree of liquid temperature from 20 °C.
const POINTS: usize = 40;
const PUMP_FLOOR: u8 = 25;
/// Curves left behind on release, from 20 to 59 °C (ours, not the plugin's).
const PUMP_RAMP: (u8, u8) = (60, 100);
const FAN_RAMP: (u8, u8) = (30, 100);

#[derive(Clone, Copy)]
struct Model {
    pid: u16,
    name: &'static str,
    interface: i32,
    len: usize,
    /// Asks for status with `74 01`; otherwise the cooler reports by itself.
    asks: bool,
    reply: u8,
    pump_head: [u8; 2],
    fan_head: Option<[u8; 2]>,
    /// Firmware 2.1.1 and later move to `75 02` replies and `01 xx` curve headers.
    firmware_gate: bool,
}

const fn model(pid: u16, name: &'static str, interface: i32, len: usize) -> Model {
    Model {
        pid,
        name,
        interface,
        len,
        asks: true,
        reply: 0x01,
        pump_head: [0, 0],
        fan_head: Some([0, 0]),
        firmware_gate: false,
    }
}

const X3: Model = Model {
    asks: false,
    reply: 0x02,
    fan_head: None,
    ..model(0x2007, "NZXT Kraken X3", 0, 64)
};
const K2023: Model = Model {
    firmware_gate: true,
    ..model(0x300E, "NZXT Kraken", 1, 64)
};

const MODELS: [Model; 7] = [
    X3,
    Model {
        pid: 0x2014,
        name: "NZXT Kraken X3 RGB",
        ..X3
    },
    model(0x3008, "NZXT Kraken Z3", 1, 64),
    K2023,
    Model {
        pid: 0x3014,
        name: "NZXT Kraken Plus",
        ..K2023
    },
    Model {
        pid: 0x300C,
        name: "NZXT Kraken Elite",
        ..K2023
    },
    Model {
        fan_head: Some([0x01, 0x01]),
        ..model(0x3012, "NZXT Kraken Elite V2", 1, 512)
    },
];

pub fn ids() -> Vec<(u16, u16, &'static str)> {
    MODELS.iter().map(|m| (VID, m.pid, m.name)).collect()
}

/// A flat curve: the same duty at every temperature point.
pub fn curve(target: u8, head: [u8; 2], duty: [u8; POINTS]) -> Vec<u8> {
    let mut p = vec![CURVE, target, head[0], head[1]];
    p.extend_from_slice(&duty);
    p
}

pub fn ramp((lo, hi): (u8, u8)) -> [u8; POINTS] {
    std::array::from_fn(|i| lo + ((hi - lo) as usize * i / (POINTS - 1)) as u8)
}

#[derive(Debug, Default, PartialEq)]
pub struct Status {
    pub liquid: f32,
    pub pump: u32,
    pub fan: u32,
}

pub fn status(r: &[u8]) -> Status {
    let at = |i: usize| r.get(i).copied().unwrap_or(0);
    Status {
        liquid: at(15) as f32 + at(16) as f32 / 10.0,
        pump: le16(r, 17),
        fan: le16(r, 23),
    }
}

/// Firmware 2.1.1 or later, from a `11 02` reply.
pub fn new_firmware(r: &[u8]) -> bool {
    r.len() > 18 && (r[16], r[17], r[18]) >= (2, 1, 1)
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    m: Model,
    status: Option<Status>,
    seen: bool,
    poll: Every,
}

impl Hub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        let (len, kind) = (self.m.len, self.m.reply);
        let want = move |r: &[u8]| r.len() > 25 && r[0] == STATUS && r[1] == kind;
        let r = if self.m.asks {
            drain(self.io.as_mut(), len);
            match self.io.write(&STATUS_ASK, len) {
                Ok(()) => read_until(self.io.as_mut(), len, 300, want).ok(),
                Err(_) => None,
            }
        } else {
            latest(self.io.as_mut(), len, want)
                .or_else(|| read_until(self.io.as_mut(), len, 1200, want).ok())
        };
        if let Some(r) = r {
            self.status = Some(status(&r));
        }
    }
    fn write_curve(&mut self, ch: usize, duty: [u8; POINTS]) -> Result<(), String> {
        let (target, head) = if ch == 0 {
            (PUMP, self.m.pump_head)
        } else {
            (FAN, self.m.fan_head.ok_or("no fan channel")?)
        };
        self.io.write(&curve(target, head, duty), self.m.len)
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.m.name.into()
    }
    fn channels(&self) -> Vec<Channel> {
        let mut c = vec![Channel::pump("Pump")];
        if self.m.fan_head.is_some() {
            c.push(Channel::fan("Fan"));
        }
        c
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Liquid".into()]
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        let s = self.status.as_ref()?;
        if ch == 0 {
            Some(s.pump)
        } else {
            spun(&mut self.seen, s.fan)
        }
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        let mut pct = percent(duty);
        if ch == 0 {
            pct = pct.max(PUMP_FLOOR);
        }
        self.write_curve(ch, [pct; POINTS])
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.write_curve(ch, ramp(if ch == 0 { PUMP_RAMP } else { FAN_RAMP }))
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        self.refresh();
        self.status.as_ref().map(|s| s.liquid).filter(|t| *t > 0.0)
    }
}

fn model_of(info: &Info) -> Option<Model> {
    MODELS
        .iter()
        .copied()
        .find(|m| info.vid == VID && info.pid == m.pid && iface(info, m.interface))
}

/// Kraken 2023 firmware decides the reply kind and curve headers.
fn upgrade(io: &mut dyn Io, m: &mut Model) {
    drain(io, m.len);
    if io.write(&FIRMWARE_ASK, m.len).is_err() {
        return;
    }
    let reply = read_until(io, m.len, 500, |r| r.len() > 18 && r[..2] == FIRMWARE);
    if reply.is_ok_and(|r| new_firmware(&r)) {
        m.reply = 0x02;
        m.pump_head = [0x01, 0x00];
        m.fan_head = Some([0x01, 0x01]);
    }
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    scan(api, seen, |i| model_of(i).is_some())
        .into_iter()
        .filter_map(|info| {
            let mut m = model_of(&info)?;
            let mut io = open(api, &info).ok()?;
            if m.firmware_gate {
                upgrade(io.as_mut(), &mut m);
            }
            Some(backend_on(
                &info,
                Hub {
                    io,
                    id: hub_id(&info, "nzxt-kraken"),
                    m,
                    status: None,
                    seen: false,
                    poll: Every::ms(1000),
                },
            ))
        })
        .collect()
}

/// Kraken Z3 accessory channel: up to 40 GRB LEDs, the same packets as the Hue 2 controllers.
const Z3_LEDS: usize = 40;
const PER_PACKET: usize = 20;

/// The count byte is `colors.len()`, up to 40.
pub fn z3_frame(colors: &[Color]) -> Vec<Vec<u8>> {
    let colors = &colors[..colors.len().min(Z3_LEDS)];
    let mut out: Vec<Vec<u8>> = colors
        .chunks(PER_PACKET)
        .enumerate()
        .map(|(n, chunk)| {
            let mut p = vec![0x22, 0x10 | n as u8, 0x01, 0x00];
            chunk
                .iter()
                .for_each(|c| p.extend_from_slice(&[c.1, c.0, c.2]));
            p
        })
        .collect();
    out.push(vec![
        0x22,
        0xA0,
        0x01,
        0x00,
        0x01,
        0x00,
        0x00,
        colors.len() as u8,
        0x00,
        0x00,
        0x80,
        0x00,
        0x32,
        0x00,
        0x00,
        0x01,
    ]);
    out
}

struct Z3Lights(usize);

impl Proto for Z3Lights {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        z3_frame(&span(colors, 0, self.0))
            .iter()
            .try_for_each(|p| io.write(p, 64))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        for p in z3_frame(&vec![Color(0, 0, 0); self.0]) {
            let _ = io.write(&p, 64);
        }
    }
}

fn open_z3(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let id = info.device_id("nzxt-kraken-z3");
    let n = foxden_hid::zone_length(&id, 0, "Accessory", Z3_LEDS, 0, Z3_LEDS);
    let mut dev = Device::grid(
        &id,
        d.name,
        DeviceKind::Strip,
        [0.05, 0.30, 0.60, 0.04],
        1,
        1,
    );
    set_rows(&mut dev, &[n]);
    Ok(Driver::new(open_io(api, info)?, dev, "nzxt-kraken-z3", Z3Lights(n)).boxed())
}

pub static DETECTORS: [Detector; 1] = [crate::hid::on_i("NZXT Kraken Z3", VID, 0x3008, 1, open_z3)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        let p = curve(PUMP, [0, 0], [40; POINTS]);
        assert_eq!(p.len(), 44);
        assert_eq!(&p[..5], &[0x72, 1, 0, 0, 40]);
        let r = ramp(PUMP_RAMP);
        assert_eq!((r[0], r[39]), (60, 100));
        let f = z3_frame(&[Color(1, 2, 3); 40]);
        assert_eq!(f.len(), 3);
        assert_eq!(&f[1][..7], &[0x22, 0x11, 1, 0, 2, 1, 3]);
        assert_eq!(f[0].len(), 64);
        assert_eq!(f[2][7], 40);
    }

    #[test]
    fn z3_count_follows_the_length() {
        let c: Vec<Color> = (0..25).map(|i| Color(i, 0, 0)).collect();
        let f = z3_frame(&c);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].len(), 4 + 20 * 3);
        assert_eq!(f[1][..7], [0x22, 0x11, 1, 0, 0, 20, 0]);
        assert_eq!(f[1].len(), 4 + 5 * 3);
        assert_eq!(f[2][7], 25);
        let f = z3_frame(&c[..8]);
        assert_eq!((f.len(), f[1][1], f[1][7]), (2, 0xA0, 8));
        assert_eq!(z3_frame(&[Color(0, 0, 0); 50])[2][7], 40);
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 64];
        r[..2].copy_from_slice(&[0x75, 0x01]);
        r[15] = 30;
        r[16] = 2;
        r[17] = 0xB8;
        r[18] = 0x0B;
        r[23] = 0x20;
        r[24] = 0x03;
        assert_eq!(
            status(&r),
            Status {
                liquid: 30.2,
                pump: 3000,
                fan: 800
            }
        );
        let mut fw = vec![0u8; 64];
        fw[16..19].copy_from_slice(&[2, 1, 1]);
        assert!(new_firmware(&fw));
        fw[17] = 0;
        assert!(!new_firmware(&fw));
    }
}
