//! Nuvoton keyboards and the Lian Li TL fan controller on 0416:7372 (protocol learned from SignalRGB's
//! Nuvoton/Nuvoton_Controller.js).

use super::nuvoton_layouts::MODELS;
use super::util::{Every, be16, cells, hub_id, open, scan};
use crate::fan::{Channel, FanHub, backend, percent};
use crate::hid::{Driver, Io, Proto, drain, open_io, read_until, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x0416;
const PID: u16 = 0x7372;
const INTERFACE: i32 = 2;
const PAGE: u16 = 0xFF1B;
const USAGE: u16 = 0x0091;
const REPORT: u8 = 0x01;
const LEN: usize = 65;
const IDENTIFY: u8 = 0x0D;
const COLORS: u8 = 0x0F;
const LEDS_PER_PACKET: usize = 18;
const TL_MODEL: &str = "TL12028-Controller";
const TL_NAME: &str = "Lian Li TL Fan Controller";
const TL_RPM: u8 = 0xA1;
const TL_SPEED: u8 = 0xAA;
const TL_PORTS: usize = 4;
const SAFE: u8 = 100;
/// The one model that needs an extra packet after each frame.
const AK820_MAX: &str = "SG8994HERGB";

pub struct Layout {
    pub size: [u8; 2],
    /// Per zone, the buffer slot of each LED, in `at` order.
    pub zones: &'static [&'static [u16]],
    pub at: &'static [[u8; 2]],
}

pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub layout: &'static Layout,
}

pub const IDS: &[(u16, u16, &str)] = &[(VID, PID, TL_NAME)];

fn fits(info: &Info) -> bool {
    info.vid == VID
        && info.pid == PID
        && info.interface == INTERFACE
        && info.usage_page == PAGE
        && info.usage == USAGE
}

/// The model id: field 4 of the comma separated identify reply.
pub fn model_id(reply: &[u8]) -> Option<String> {
    let text: String = reply.iter().map(|&b| b as char).collect();
    text.split(',')
        .nth(4)
        .map(|s| s.trim_matches(char::from(0)).trim().to_string())
}

fn identify(io: &mut dyn Io) -> Result<String, String> {
    drain(io, LEN);
    io.write(&[REPORT, IDENTIFY], LEN)?;
    let r = read_until(io, LEN, 500, |r| {
        r.iter().filter(|&&b| b == b',').count() >= 5
    })?;
    model_id(&r).ok_or_else(|| "no model id".into())
}

/// TL packets: report, opcode, three zeros, payload length, payload.
pub fn tl_packet(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![REPORT, opcode, 0, 0, 0, payload.len() as u8];
    p.extend_from_slice(payload);
    p
}

pub fn tl_speed(port: usize, fan: u8, pct: u8) -> Vec<u8> {
    tl_packet(TL_SPEED, &[(port as u8) << 4 | (fan & 0x0F), pct])
}

/// `A1` reply: (port, fan index, rpm) for every fan the controller sees.
pub fn tl_fans(r: &[u8]) -> Vec<(usize, u8, u32)> {
    let Some(&n) = r.get(5) else {
        return Vec::new();
    };
    (0..n as usize / 3)
        .filter_map(|i| {
            let at = 6 + i * 3;
            let info = *r.get(at + 2).and(r.get(at))?;
            (info & 0x80 != 0)
                .then(|| (((info >> 4) & 0x03) as usize, info & 0x0F, be16(r, at + 1)))
        })
        .collect()
}

struct TlHub {
    io: Box<dyn Io>,
    id: String,
    /// Fan indexes seen on each port, as a bit mask.
    fans: [u16; TL_PORTS],
    rpm: [Option<u32>; TL_PORTS],
    poll: Every,
}

impl TlHub {
    fn refresh(&mut self) {
        if !self.poll.due() {
            return;
        }
        drain(self.io.as_mut(), LEN);
        if self.io.write(&[REPORT, TL_RPM], LEN).is_err() {
            return;
        }
        let Ok(r) = read_until(self.io.as_mut(), LEN, 300, |r| {
            r.len() >= 6 && r[0] == REPORT
        }) else {
            return;
        };
        self.rpm = [None; TL_PORTS];
        for (port, fan, rpm) in tl_fans(&r) {
            self.fans[port] |= 1 << fan;
            self.rpm[port] = Some(self.rpm[port].unwrap_or(0).max(rpm));
        }
    }
    fn send(&mut self, port: usize, pct: u8) -> Result<(), String> {
        let mask = if self.fans[port] == 0 {
            1
        } else {
            self.fans[port]
        };
        for fan in (0..16u8).filter(|f| mask & 1 << f != 0) {
            self.io.write(&tl_speed(port, fan, pct), LEN)?;
        }
        Ok(())
    }
}

impl FanHub for TlHub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        TL_NAME.into()
    }
    fn channels(&self) -> Vec<Channel> {
        (1..=TL_PORTS)
            .map(|i| Channel::fan(format!("Port {i}")))
            .collect()
    }
    fn rpm(&mut self, ch: usize) -> Option<u32> {
        self.refresh();
        *self.rpm.get(ch)?
    }
    fn set_duty(&mut self, ch: usize, duty: f32) -> Result<(), String> {
        self.send(ch, percent(duty))
    }
    fn release(&mut self, ch: usize) -> Result<(), String> {
        self.send(ch, SAFE)
    }
}

pub fn backends(api: &HidApi, seen: &mut Vec<String>) -> Vec<Box<dyn FanBackend>> {
    let mut out = Vec::new();
    for info in scan(api, &mut Vec::new(), fits) {
        if seen.contains(&info.key()) {
            continue;
        }
        let Ok(mut io) = open(api, &info) else {
            continue;
        };
        if identify(io.as_mut()).ok().as_deref() != Some(TL_MODEL) {
            continue;
        }
        seen.push(info.key());
        out.push(backend(TlHub {
            io,
            id: hub_id(&info, "lianli-tl"),
            fans: [0; TL_PORTS],
            rpm: [None; TL_PORTS],
            poll: Every::ms(1500),
        }));
    }
    out
}

/// Direct lighting mode, as the keyboards' own software sets it.
const DIRECT: [&[u8]; 3] = [
    &[REPORT, 0x07, 0x00, 0x00, 0x00, 0x0E, 0x00, 0x04, 0x03, 0xFF],
    &[REPORT, 0x17, 0x00, 0x00, 0x00, 0x01, 0x01],
    &[
        REPORT, 0x08, 0x00, 0x00, 0x00, 0x0D, 0x02, 0x03, 0x03, 0xFF, 0, 0, 0, 0, 0, 0, 0x01,
    ],
];
/// Sent when a board has no second zone, so the firmware still gets one.
const EMPTY_ZONE: [&[u8]; 2] = [
    &[REPORT, COLORS, 0x01, 0x00, 0x00, 0x36],
    &[REPORT, COLORS, 0x01, 0x00, 0x01, 0x2D],
];

/// One frame: each zone's buffer in packets of 18 LEDs.
pub fn frame(layout: &Layout, model: &str, colors: &[Color]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut next = colors.iter();
    for (z, slots) in layout.zones.iter().enumerate() {
        let len = slots.iter().copied().max().map_or(0, |m| m as usize + 1);
        let mut buf = vec![0u8; len * 3];
        for &s in slots.iter() {
            let c = next.next().copied().unwrap_or(Color(0, 0, 0));
            buf[s as usize * 3..s as usize * 3 + 3].copy_from_slice(&rgb(c));
        }
        for (n, chunk) in buf.chunks(LEDS_PER_PACKET * 3).enumerate() {
            let mut p = vec![REPORT, COLORS, z as u8, 0x00, n as u8, chunk.len() as u8];
            p.extend_from_slice(chunk);
            out.push(p);
        }
    }
    if layout.zones.len() == 1 {
        out.extend(EMPTY_ZONE.iter().map(|p| p.to_vec()));
        if model == AK820_MAX {
            out.push(vec![REPORT, COLORS, 0x00, 0x00, 0x07, 0x12]);
        }
    }
    out
}

struct Keyboard {
    model: &'static Model,
}

impl Keyboard {
    fn send(&self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in frame(self.model.layout, self.model.id, colors) {
            io.write(&p, LEN)?;
            io.sleep(1);
        }
        Ok(())
    }
}

impl Proto for Keyboard {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        DIRECT.iter().try_for_each(|p| io.write(p, LEN))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.send(io, colors)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.send(io, &vec![Color(0, 0, 0); self.model.layout.at.len()]);
    }
}

fn open_keyboard(api: &HidApi, info: &Info, _d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    let id = identify(io.as_mut())?;
    if id == TL_MODEL {
        return Err("the Lian Li TL controller is fan-only".into());
    }
    let model = MODELS
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("unknown Nuvoton model {id}"))?;
    let l = model.layout;
    let dev = cells(
        &info.device_id("nuvoton"),
        model.name,
        DeviceKind::Keyboard,
        [0.05, 0.55, 0.60, 0.25],
        [l.size[0] as u32, l.size[1] as u32],
        l.at,
    );
    Ok(Driver::new(io, dev, "nuvoton", Keyboard { model }).boxed())
}

pub static DETECTORS: [Detector; 1] = [Detector {
    name: "Nuvoton Keyboard",
    vid: VID,
    pid: PID,
    interface: Some(INTERFACE),
    usage_page: Some(PAGE),
    usage: Some(USAGE),
    open: open_keyboard,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tl() {
        assert_eq!(tl_speed(2, 1, 70), [1, 0xAA, 0, 0, 0, 2, 0x21, 70]);
        let r = [1, 0xA1, 0, 0, 0, 6, 0x90, 0x07, 0x80, 0x01, 0, 0];
        assert_eq!(tl_fans(&r), vec![(1, 0, 1920)]);
        let id = b"\x01\x0dV1,x,y,z,TL12028-Controller,1.0\0\0";
        assert_eq!(model_id(id).as_deref(), Some(TL_MODEL));
    }

    #[test]
    fn keyboard_frames() {
        for m in MODELS.iter() {
            let n: usize = m.layout.zones.iter().map(|z| z.len()).sum();
            assert_eq!(n, m.layout.at.len(), "{}", m.id);
        }
        let full = MODELS.iter().find(|m| m.id == "GK5000RGB").unwrap();
        let f = frame(
            full.layout,
            full.id,
            &vec![Color(9, 8, 7); full.layout.at.len()],
        );
        assert_eq!(f.len(), 8 + 2);
        assert_eq!(&f[0][..9], &[1, 0x0F, 0, 0, 0, 54, 9, 8, 7]);
        assert_eq!(&f[0][9..12], &[0, 0, 0]);
        assert_eq!(f[7][5], 5 * 3);
        assert_eq!(f[8], [1, 0x0F, 1, 0, 0, 0x36]);
    }
}
