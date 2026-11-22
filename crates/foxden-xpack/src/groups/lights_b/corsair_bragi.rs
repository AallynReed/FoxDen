//! Corsair Bragi headsets and their slipstream dongles: Virtuoso Max, Void Wireless V2, HS80 Max.

use std::time::Instant;

use super::{HEADSET, boxed, det, placed};
use crate::hid::{Io, Proto, drain, read_until, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "corsair-bragi";
const VID: u16 = 0x1B1C;
const REPORT: u8 = 0x02;
const LEN: usize = 64;
const REPLY_MS: u64 = 200;

const SET: u8 = 0x01;
const GET: u8 = 0x02;
const CLOSE: u8 = 0x05;
const WRITE: u8 = 0x06;
const STREAM: u8 = 0x07;
const CHECK: u8 = 0x09;
const OPEN: u8 = 0x0D;
const PING: u8 = 0x12;

const BRIGHTNESS: u16 = 0x02;
const MODE: u16 = 0x03;
const CHILDREN: u16 = 0x36;
const HARDWARE: u32 = 0x01;
const SOFTWARE: u32 = 0x02;
const FULL_BRIGHTNESS: u32 = 1000;

const LIGHTING_HANDLE: u8 = 0x00;
const BACKGROUND_HANDLE: u8 = 0x01;
const AUX_HANDLE: u8 = 0x02;
const LIGHTING: u8 = 0x01;
const LIGHTING_CONTROLLER: u8 = 0x22;
/// Data type the lighting-controller endpoint expects before the RGB triplets.
const CONTROLLER_HEAD: [u8; 2] = [0x12, 0x00];
/// Planar data keeps three slots per colour plane.
const SPACING: usize = 3;
/// iCUE pings about every 52 s; this keeps the headset in software mode.
const PING_MS: u64 = 50_000;

struct Model {
    size: [u32; 2],
    pos: &'static [[u8; 2]],
    map: &'static [u8],
}

const VIRTUOSO_MAX: Model = Model {
    size: [9, 3],
    pos: &[[2, 0], [0, 2], [3, 2], [7, 0], [5, 2], [8, 2]],
    map: &[0, 1, 2, 3, 4, 5],
};
const VOID_V2: Model = Model {
    size: [2, 2],
    pos: &[[0, 0], [0, 1]],
    map: &[0, 1],
};
const HS80_MAX: Model = Model {
    size: [3, 3],
    pos: &[[1, 0], [2, 2]],
    map: &[0, 1],
};

const PIDS: &[(u16, &Model)] = &[
    (0x2A00, &VIRTUOSO_MAX),
    (0x2A02, &VIRTUOSO_MAX),
    (0x2A08, &VOID_V2),
    (0x2A09, &VOID_V2),
    (0x0A96, &HS80_MAX),
    (0x0A97, &HS80_MAX),
    (0x2A0E, &HS80_MAX),
    (0x2A0F, &HS80_MAX),
];

const fn bragi(pid: u16, name: &'static str) -> Detector {
    det(name, VID, pid, None, Some(0xFF42), Some(0x01), open)
}

pub static DETECTORS: &[Detector] = &[
    bragi(0x2A00, "Corsair Virtuoso Max"),
    bragi(0x2A02, "Corsair Virtuoso Max"),
    bragi(0x2A08, "Corsair Void Wireless V2"),
    bragi(0x2A09, "Corsair Void Wireless V2"),
    bragi(0x0A96, "Corsair HS80 Max"),
    bragi(0x0A97, "Corsair HS80 Max"),
    bragi(0x2A0E, "Corsair HS80 Max"),
    bragi(0x2A0F, "Corsair HS80 Max"),
];

/// Every command: report 2, the device (0 wired or dongle, 1..7 a dongle's child) with bit 3, the command.
pub fn request(dev: u8, cmd: u8, args: &[u8]) -> Vec<u8> {
    let mut b = vec![REPORT, 0x08 | dev, cmd];
    b.extend_from_slice(args);
    b
}

/// `SendRGBData` payload: R, G, B planes three slots apart, or controller triplets.
pub fn payload(map: &[u8], colors: &[Color], controller: bool) -> Vec<u8> {
    if controller {
        let mut out = vec![0u8; map.len() * 3];
        for (&i, &c) in map.iter().zip(colors) {
            let i = i as usize * 3;
            out[i..i + 3].copy_from_slice(&rgb(c));
        }
        let mut head = CONTROLLER_HEAD.to_vec();
        head.extend(out);
        return head;
    }
    let top = map.iter().map(|&i| i as usize).max().unwrap_or(0);
    let mut out = vec![0u8; (top + 2 * SPACING + 1).max(3 * SPACING)];
    for (&i, &c) in map.iter().zip(colors) {
        let i = i as usize;
        out[i] = c.0;
        out[i + SPACING] = c.1;
        out[i + 2 * SPACING] = c.2;
    }
    out
}

/// `WriteLighting` then `StreamLighting`: the first packet carries the total length.
pub fn lighting_packets(dev: u8, data: &[u8]) -> Vec<Vec<u8>> {
    let first = data.len().min(LEN - 8);
    let mut head = request(dev, WRITE, &[LIGHTING_HANDLE]);
    head.extend((data.len() as u32).to_le_bytes());
    head.extend_from_slice(&data[..first]);
    let mut out = vec![head];
    for chunk in data[first..].chunks(LEN - 4) {
        let mut p = vec![REPORT, dev, STREAM, LIGHTING_HANDLE];
        p.extend_from_slice(chunk);
        out.push(p);
    }
    out
}

struct Bragi {
    map: &'static [u8],
    dev: u8,
    controller: bool,
    pinged: Instant,
}

impl Bragi {
    fn ask(&self, io: &mut dyn Io, dev: u8, cmd: u8, args: &[u8]) -> Result<Vec<u8>, String> {
        drain(io, LEN);
        io.write(&request(dev, cmd, args), LEN)?;
        read_until(io, LEN, REPLY_MS, |r| r.len() > 3 && r[2] == cmd)
    }

    fn property(&self, io: &mut dyn Io, dev: u8, id: u16) -> Option<u32> {
        let r = self.ask(io, dev, GET, &id.to_le_bytes()).ok()?;
        (r[3] == 0).then(|| {
            let b = |i: usize| r.get(i).copied().unwrap_or(0) as u32;
            b(4) | b(5) << 8 | b(6) << 16
        })
    }

    fn set(&self, io: &mut dyn Io, dev: u8, id: u16, v: u32) -> Result<(), String> {
        let [a, b, c, _] = v.to_le_bytes();
        self.ask(io, dev, SET, &[id as u8, 0x00, a, b, c])
            .map(|_| ())
    }

    fn close_if_open(&self, io: &mut dyn Io, dev: u8, handle: u8) {
        if self
            .ask(io, dev, CHECK, &[handle, 0x00])
            .is_ok_and(|r| r[3] != 3)
        {
            let _ = self.ask(io, dev, CLOSE, &[0x01, handle]);
        }
    }

    fn open_handle(&self, io: &mut dyn Io, dev: u8, handle: u8, endpoint: u8) -> bool {
        self.ask(io, dev, OPEN, &[handle, endpoint])
            .is_ok_and(|r| r[3] == 0)
    }

    fn set_mode(&self, io: &mut dyn Io, dev: u8, mode: u32) -> Result<(), String> {
        let now = self.property(io, dev, MODE);
        for h in [LIGHTING_HANDLE, BACKGROUND_HANDLE, AUX_HANDLE] {
            self.close_if_open(io, dev, h);
        }
        if now != Some(mode) {
            self.set(io, dev, MODE, mode)?;
        }
        Ok(())
    }

    fn endpoint(&self) -> u8 {
        if self.controller {
            LIGHTING_CONTROLLER
        } else {
            LIGHTING
        }
    }
}

impl Proto for Bragi {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        self.set_mode(io, 0, SOFTWARE)?;
        self.dev = 0;
        if let Some(mask) = self.property(io, 0, CHILDREN) {
            self.dev = (1..8u8)
                .find(|i| mask & (1 << i) != 0)
                .ok_or("no headset paired to the dongle")?;
            self.set_mode(io, self.dev, SOFTWARE)?;
        }
        let dev = self.dev;
        if self.property(io, dev, BRIGHTNESS) != Some(FULL_BRIGHTNESS) {
            self.set(io, dev, BRIGHTNESS, FULL_BRIGHTNESS)?;
        }
        self.close_if_open(io, dev, BACKGROUND_HANDLE);
        self.controller = if self.open_handle(io, dev, BACKGROUND_HANDLE, LIGHTING) {
            false
        } else if self.open_handle(io, dev, BACKGROUND_HANDLE, LIGHTING_CONTROLLER) {
            true
        } else {
            return Err("no lighting endpoint".into());
        };
        let _ = self.ask(io, dev, CLOSE, &[0x01, BACKGROUND_HANDLE]);
        self.open_handle(io, dev, LIGHTING_HANDLE, self.endpoint());
        self.pinged = Instant::now();
        Ok(())
    }

    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let dev = self.dev;
        if self.pinged.elapsed().as_millis() as u64 >= PING_MS {
            let _ = self.ask(io, dev, PING, &[]);
            if self.property(io, dev, MODE) != Some(SOFTWARE) {
                self.set_mode(io, dev, SOFTWARE)?;
                self.open_handle(io, dev, LIGHTING_HANDLE, self.endpoint());
            }
            self.pinged = Instant::now();
        }
        let packets = lighting_packets(dev, &payload(self.map, colors, self.controller));
        drain(io, LEN);
        io.write(&packets[0], LEN)?;
        io.sleep(3);
        let reply = io.read(LEN, REPLY_MS as i32)?;
        if reply.get(3).is_some_and(|&e| e != 0) {
            io.sleep(50);
            self.open_handle(io, dev, LIGHTING_HANDLE, self.endpoint());
        }
        for p in &packets[1..] {
            io.write(p, LEN)?;
            io.sleep(1);
        }
        Ok(())
    }

    fn keepalive_ms(&self) -> Option<u64> {
        Some(PING_MS)
    }

    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.set_mode(io, self.dev, HARDWARE);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let model = PIDS
        .iter()
        .find(|p| p.0 == info.pid)
        .map(|p| p.1)
        .ok_or("unknown Bragi headset")?;
    let dev = placed(
        info,
        FAMILY,
        d.name,
        DeviceKind::Other,
        HEADSET,
        model.size,
        model.pos,
    );
    let proto = Bragi {
        map: model.map,
        dev: 0,
        controller: false,
        pinged: Instant::now(),
    };
    boxed(api, info, "corsair bragi", dev, proto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_and_controller_payloads() {
        let c = [Color(1, 2, 3), Color(4, 5, 6)];
        assert_eq!(payload(&[0, 1], &c, false), [1, 4, 0, 2, 5, 0, 3, 6, 0]);
        assert_eq!(payload(&[0, 1], &c, true), [0x12, 0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn length_prefixed_then_streamed() {
        let data: Vec<u8> = (0..70).collect();
        let p = lighting_packets(2, &data);
        assert_eq!(p[0][..8], [2, 0x0A, 0x06, 0, 70, 0, 0, 0]);
        assert_eq!(p[0].len(), 64);
        assert_eq!(p[1][..5], [2, 2, 0x07, 0, 56]);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn property_request() {
        assert_eq!(request(0, GET, &MODE.to_le_bytes()), [2, 8, 2, 3, 0]);
    }
}
