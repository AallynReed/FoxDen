//! Corsair Bragi peripherals: keyboards, mice and their Slipstream dongles.

use super::bragi_models::{MODELS, Model, PIDS};
use super::util::due;
use crate::hid::{Driver, Io, Proto, drain, open_io, read_until};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;
use std::time::Instant;

const VID: u16 = 0x1B1C;
const PAGE: u16 = 0xFF42;
const SET_PROPERTY: u8 = 0x01;
const GET_PROPERTY: u8 = 0x02;
const CLOSE: u8 = 0x05;
const WRITE: u8 = 0x06;
const STREAM: u8 = 0x07;
const OPEN: u8 = 0x0D;
const PING: u8 = 0x12;
const PROP_BRIGHTNESS: u8 = 0x02;
const PROP_MODE: u8 = 0x03;
const PROP_PID: u8 = 0x12;
const PROP_CHILDREN: u8 = 0x36;
const HARDWARE: u32 = 0x01;
const SOFTWARE: u32 = 0x02;
const FULL_BRIGHTNESS: u32 = 1000;
/// The K100 Air reports full brightness as 999.
const K100_AIR: u16 = 0x1BAB;
const EP_LIGHTING: u8 = 0x01;
const EP_CONTROLLER: u8 = 0x22;
const CONTROLLER_TYPE: [u8; 2] = [0x12, 0x00];
const BACKGROUND: u8 = 0x01;
/// Command flag a V1 header sets when it wants a reply.
const WANTS_REPLY: u8 = 0x08;
/// Probe writes fit every Bragi report; the reply length gives the real size.
const PROBE_LEN: usize = 65;
const MAX_LEN: usize = 1025;
const REPLY_MS: u64 = 100;
const MODE_POLL_MS: u64 = 5000;
const PING_MS: u64 = 50_000;

/// Header layout: V1 `00 dev|08 cmd`, V2 `00 00 01 dev cmd`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub v2: bool,
    pub dev: u8,
    pub size: usize,
}

impl Link {
    /// Header bytes including the report id; replies carry one fewer.
    pub fn hlen(&self) -> usize {
        if self.v2 { 5 } else { 3 }
    }

    pub fn header(&self, cmd: u8, reply: bool) -> Vec<u8> {
        if self.v2 {
            vec![0x00, 0x00, 0x01, self.dev, cmd]
        } else {
            vec![
                0x00,
                if reply {
                    self.dev | WANTS_REPLY
                } else {
                    self.dev
                },
                cmd,
            ]
        }
    }

    fn call(&self, io: &mut dyn Io, cmd: u8, args: &[u8]) -> Option<Vec<u8>> {
        let mut p = self.header(cmd, true);
        p.extend_from_slice(args);
        drain(io, MAX_LEN);
        io.write(&p, self.size).ok()?;
        let echo = self.hlen() - 2;
        let r = read_until(io, MAX_LEN, REPLY_MS, |r| r.get(echo) == Some(&cmd)).ok()?;
        (r.get(self.hlen() - 1) == Some(&0)).then_some(r)
    }

    pub fn value(&self, reply: &[u8]) -> Option<u32> {
        let v = reply.get(self.hlen()..self.hlen() + 3)?;
        Some(u32::from(v[0]) | u32::from(v[1]) << 8 | u32::from(v[2]) << 16)
    }

    fn get(&self, io: &mut dyn Io, prop: u8) -> Option<u32> {
        self.value(&self.call(io, GET_PROPERTY, &[prop, 0x00])?)
    }

    fn set(&self, io: &mut dyn Io, prop: u8, value: u32) -> bool {
        let v = value.to_le_bytes();
        self.call(io, SET_PROPERTY, &[prop, 0x00, v[0], v[1], v[2]])
            .is_some()
    }

    fn open(&self, io: &mut dyn Io, handle: u8, endpoint: u8) -> bool {
        self.call(io, OPEN, &[handle, endpoint]).is_some()
    }

    fn close(&self, io: &mut dyn Io, handle: u8) {
        let _ = self.call(io, CLOSE, &[0x01, handle]);
    }
}

/// `getLightingControllerColors`: `12 00`, then RGB triplets at each LED's map index.
pub fn controller_data(model: &Model, colors: &[Color]) -> Vec<u8> {
    let top = model.map.iter().map(|&m| m as usize + 1).max().unwrap_or(0);
    let mut d = vec![0u8; 3 * model.map.len().max(top)];
    for (i, c) in colors.iter().enumerate().take(model.pos.len()) {
        let at = model.map[i] as usize * 3;
        d[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    let mut out = CONTROLLER_TYPE.to_vec();
    out.extend(d);
    out
}

/// `getStandardColors`: red, green and blue planes `spacing` bytes apart.
pub fn planar_data(model: &Model, colors: &[Color]) -> Vec<u8> {
    let s = model.spacing as usize;
    let top = model
        .map
        .iter()
        .map(|&m| m as usize + 2 * s + 1)
        .max()
        .unwrap_or(0);
    let mut d = vec![0u8; (3 * s).max(top)];
    for (i, c) in colors.iter().enumerate().take(model.pos.len()) {
        let at = model.map[i] as usize;
        d[at] = c.0;
        d[at + s] = c.1;
        d[at + 2 * s] = c.2;
    }
    d
}

/// `SendRGBData`: the first packet carries the total length, the rest stream on.
pub fn rgb_packets(link: &Link, handle: u8, data: &[u8], stream_reply: bool) -> Vec<Vec<u8>> {
    let first = link.size - (link.hlen() + 5);
    let rest = link.size - (link.hlen() + 1);
    let mut p = link.header(WRITE, true);
    p.push(handle);
    p.extend((data.len() as u32).to_le_bytes());
    let n = first.min(data.len());
    p.extend_from_slice(&data[..n]);
    let mut out = vec![p];
    for chunk in data[n..].chunks(rest) {
        let mut s = link.header(STREAM, stream_reply);
        s.push(handle);
        s.extend_from_slice(chunk);
        out.push(s);
    }
    out
}

pub fn model_of(pid: u16) -> Option<&'static Model> {
    let key = PIDS.iter().find(|p| p.0 == pid)?.1;
    MODELS.iter().find(|m| m.key == key)
}

/// The protocol version, the buffer size, and the device to address (a dongle's child).
fn identify(io: &mut dyn Io, usb_pid: u16) -> Result<(Link, &'static Model, u16), String> {
    let mut found = None;
    for v2 in [false, true] {
        let probe = Link {
            v2,
            dev: 0,
            size: PROBE_LEN,
        };
        if let Some(r) = probe.call(io, GET_PROPERTY, &[PROP_PID, 0x00]) {
            let link = Link {
                size: (r.len() + 1).clamp(PROBE_LEN, MAX_LEN),
                ..probe
            };
            found = Some((link, link.value(&r).unwrap_or(0) as u16));
            break;
        }
    }
    let (mut link, mut pid) = found.ok_or("no answer")?;
    if let Some(mask) = link.get(io, PROP_CHILDREN) {
        let child = (1..8u8)
            .find(|i| mask & 1 << i != 0)
            .ok_or("no paired device")?;
        link.dev = child;
        pid = (0..3)
            .find_map(|_| link.get(io, PROP_PID))
            .ok_or("paired device silent")? as u16;
    }
    let model = model_of(pid)
        .or_else(|| model_of(usb_pid))
        .filter(|m| !m.pos.is_empty())
        .ok_or("no LEDs")?;
    Ok((link, model, pid))
}

struct Lights {
    link: Link,
    model: &'static Model,
    pid: u16,
    usb_pid: u16,
    controller: bool,
    opened: bool,
    mode_at: Option<Instant>,
    ping_at: Option<Instant>,
}

impl Lights {
    fn software(&mut self, io: &mut dyn Io) {
        self.mode_at = Some(Instant::now());
        if self.link.get(io, PROP_MODE) != Some(SOFTWARE) {
            for h in 0..3 {
                self.link.close(io, h);
            }
            self.link.set(io, PROP_MODE, SOFTWARE);
            self.opened = false;
        }
    }
}

impl Proto for Lights {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        self.software(io);
        let full = if self.pid == K100_AIR || self.usb_pid == K100_AIR {
            FULL_BRIGHTNESS - 1
        } else {
            FULL_BRIGHTNESS
        };
        if self.link.get(io, PROP_BRIGHTNESS) != Some(full) {
            self.link.set(io, PROP_BRIGHTNESS, full);
        }
        self.controller = self.model.controller.unwrap_or_else(|| {
            let ok = self.link.open(io, BACKGROUND, EP_CONTROLLER);
            if ok {
                self.link.close(io, BACKGROUND);
            }
            ok
        });
        Ok(())
    }

    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if due(self.mode_at, MODE_POLL_MS) {
            self.software(io);
        }
        if due(self.ping_at, PING_MS) {
            self.ping_at = Some(Instant::now());
            let _ = self.link.call(io, PING, &[]);
        }
        let (handle, ep) = if self.controller {
            (self.model.handle, EP_CONTROLLER)
        } else {
            (self.model.handle, EP_LIGHTING)
        };
        if !self.opened {
            self.link.close(io, handle);
            self.opened = self.link.open(io, handle, ep);
        }
        let data = if self.controller {
            controller_data(self.model, colors)
        } else {
            planar_data(self.model, colors)
        };
        let reply = self.link.v2 || matches!(self.model.key, "K70 Core" | "K60 Pro");
        for (i, p) in rgb_packets(&self.link, handle, &data, reply)
            .iter()
            .enumerate()
        {
            drain(io, MAX_LEN);
            io.write(p, self.link.size)?;
            if i == 0 || reply {
                let r = io.read(MAX_LEN, REPLY_MS as i32)?;
                if i == 0 && r.get(self.link.hlen() - 1).is_some_and(|&e| e != 0) {
                    self.opened = self.link.open(io, handle, ep);
                }
            } else {
                io.sleep(1);
            }
        }
        Ok(())
    }

    fn keepalive_ms(&self) -> Option<u64> {
        Some(2000)
    }

    fn stop(&mut self, io: &mut dyn Io) {
        self.link.close(io, self.model.handle);
        self.link.set(io, PROP_MODE, HARDWARE);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    let (link, model, pid) = identify(io.as_mut(), info.pid)?;
    let (kind, rect) = if model.keyboard {
        (DeviceKind::Keyboard, [0.06, 0.58, 0.54, 0.32])
    } else {
        (DeviceKind::Mouse, [0.70, 0.62, 0.06, 0.22])
    };
    let (w, h) = (u32::from(model.size.0), u32::from(model.size.1));
    let name = if d.name.is_empty() {
        model.name
    } else {
        d.name
    };
    let mut dev = Device::grid(&info.device_id("corsair-bragi"), name, kind, rect, w, h);
    dev.leds = model
        .pos
        .iter()
        .map(|&(x, y)| {
            [
                (f32::from(x) + 0.5) / w as f32,
                (f32::from(y) + 0.5) / h as f32,
            ]
        })
        .collect();
    let lights = Lights {
        link,
        model,
        pid,
        usb_pid: info.pid,
        controller: false,
        opened: false,
        mode_at: None,
        ping_at: None,
    };
    Ok(Driver::new(io, dev, "corsair bragi", lights).boxed())
}

const fn det(name: &'static str, pid: u16) -> Detector {
    Detector {
        name,
        vid: VID,
        pid,
        interface: Some(1),
        usage_page: Some(PAGE),
        usage: Some(0x01),
        open,
    }
}

/// The ids FoxDen has no driver for.
pub static DETECTORS: [Detector; 36] = [
    det("Corsair Slipstream Dongle", 0x0D01),
    det("Corsair K57 Wireless", 0x1B62),
    det("Corsair Sabre RGB Pro", 0x1B79),
    det("Corsair Dark Core Pro SE Wireless", 0x1B7F),
    det("Corsair Dark Core Pro Wireless", 0x1B80),
    det("Corsair Dark Core Pro", 0x1B81),
    det("Corsair Sabre RGB Pro Wireless", 0x1B98),
    det("Corsair K100 Air", 0x1BAB),
    det("Corsair Darkstar Wireless", 0x1BB2),
    det("Corsair K70 Pro Mini", 0x1BB6),
    det("Corsair Nightsabre Wireless", 0x1BB8),
    det("Corsair K65 Mini", 0x1BBD),
    det("Corsair K70 Max", 0x1BC0),
    det("Corsair K65 Mini", 0x1BC3),
    det("Corsair K65 Mini", 0x1BCF),
    det("Corsair Voyager A1600", 0x1BD0),
    det("Corsair K70 Pro Mini", 0x1BD3),
    det("Corsair K70 Pro", 0x1BD4),
    det("Corsair M65 Ultra Wireless", 0x1BD6),
    det("Corsair K65 Pro Mini", 0x1BD7),
    det("Corsair Scimitar Elite Wireless", 0x1BDB),
    det("Corsair M75 Air", 0x1BF2),
    det("Corsair K55 Core", 0x1BFE),
    det("Corsair K70 Core", 0x1BFF),
    det("Corsair K70 Core TKL Wireless", 0x2B02),
    det("Corsair MAKR75", 0x2B04),
    det("Corsair K55 Core", 0x2B05),
    det("Corsair M75 Glacier Blue", 0x2B08),
    det("Corsair K70 Core", 0x2B09),
    det("Corsair K70 Core", 0x2B0A),
    det("Corsair Galleon 100 SD", 0x2B0C),
    det("Corsair K65 Plus", 0x2B10),
    det("Corsair K65 Plus", 0x2B11),
    det("Corsair K70 Pro TKL", 0x2B14),
    det("Corsair K55 Core XT", 0x2B20),
    det("Corsair Scimitar Elite Wireless SE", 0x2B22),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers() {
        let v1 = Link {
            v2: false,
            dev: 1,
            size: 65,
        };
        assert_eq!(v1.header(GET_PROPERTY, true), vec![0x00, 0x09, 0x02]);
        assert_eq!(v1.header(STREAM, false), vec![0x00, 0x01, 0x07]);
        let v2 = Link {
            v2: true,
            dev: 0,
            size: 65,
        };
        assert_eq!(v2.header(OPEN, true), vec![0x00, 0x00, 0x01, 0x00, 0x0D]);
        assert_eq!(v1.value(&[0x00, 0x02, 0x00, 0xE8, 0x03, 0x00]), Some(1000));
        assert_eq!(
            v2.value(&[0, 0, 0, 0x02, 0x00, 0x12, 0x1B, 0x00]),
            Some(0x1B12)
        );
    }

    #[test]
    fn every_detector_has_a_model() {
        for d in &DETECTORS {
            if d.pid != 0x0D01 {
                assert!(model_of(d.pid).is_some(), "{:04x}", d.pid);
            }
        }
        for m in MODELS {
            assert!(m.map.len() >= m.pos.len(), "{}", m.key);
            assert!(m.pos.iter().all(|p| p.0 < m.size.0 && p.1 < m.size.1));
        }
    }

    #[test]
    fn controller_layout() {
        let m = model_of(0x2B05).unwrap();
        let colors = vec![Color(1, 2, 3); m.pos.len()];
        let d = controller_data(m, &colors);
        assert_eq!(&d[..2], &[0x12, 0x00]);
        assert_eq!(d.len(), 2 + 3 * 11);
        assert_eq!(&d[2..5], &[0, 0, 0]);
        assert_eq!(&d[5..8], &[1, 2, 3]);
    }

    #[test]
    fn planar_layout() {
        let m = model_of(0x1B7F).unwrap();
        let colors: Vec<Color> = (0..m.pos.len()).map(|i| Color(i as u8, 100, 200)).collect();
        let d = planar_data(m, &colors);
        let s = m.spacing as usize;
        assert_eq!(d.len(), 3 * s);
        assert_eq!(d[m.map[3] as usize], 3);
        assert_eq!(d[m.map[3] as usize + s], 100);
        assert_eq!(d[m.map[3] as usize + 2 * s], 200);
    }

    #[test]
    fn packets_split() {
        let link = Link {
            v2: false,
            dev: 0,
            size: 65,
        };
        let data: Vec<u8> = (0..200u8).collect();
        let p = rgb_packets(&link, 0, &data, false);
        assert_eq!(&p[0][..8], &[0x00, 0x08, 0x06, 0x00, 200, 0, 0, 0]);
        assert_eq!(p[0].len(), 65);
        assert_eq!(&p[1][..5], &[0x00, 0x00, 0x07, 0x00, 57]);
        assert_eq!(p.len(), 1 + (200 - 57usize).div_ceil(61));
        let v2 = Link {
            v2: true,
            dev: 0,
            size: 65,
        };
        assert_eq!(
            rgb_packets(&v2, 1, &data, true)[1][..6],
            [0, 0, 1, 0, 0x07, 0x01]
        );
    }
}
