//! Corsair Vanguard 96 keyboard, Bragi lighting controller (protocol learned from SignalRGB's Corsair_Vanguard_96.js).

use super::super::common::{KEYBOARD_RECT, full, keyed};
use super::layouts::{VANGUARD, VANGUARD_CELLS};
use crate::hid::{Driver, Io, Proto, drain, open_io};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

/// The command endpoint's fixed report size.
const LEN: usize = 1025;
const WIRED: u8 = 0x08;
const SET_PROPERTY: u8 = 0x01;
const CLOSE_HANDLE: u8 = 0x05;
const WRITE_ENDPOINT: u8 = 0x06;
const STREAM_ENDPOINT: u8 = 0x07;
const OPEN_ENDPOINT: u8 = 0x0D;
const PING: u8 = 0x12;
const PROP_BRIGHTNESS: u8 = 0x02;
const PROP_MODE: u8 = 0x03;
const MODE_HARDWARE: u32 = 0x01;
const MODE_SOFTWARE: u32 = 0x02;
/// Hardware brightness is set to its maximum, 1000.
const BRIGHTNESS_MAX: u32 = 1000;
const LIGHTING_HANDLE: u8 = 0x00;
const HANDLES: [u8; 3] = [0x00, 0x01, 0x02];
const LIGHTING_CONTROLLER: u8 = 0x22;
/// Lighting-controller data starts with its data type.
const DATA_TYPE: [u8; 2] = [0x12, 0x00];
const REPLY_MS: u64 = 100;

fn cmd(body: &[u8]) -> Vec<u8> {
    let mut p = vec![0x00, WIRED];
    p.extend_from_slice(body);
    p
}

pub fn set_property(prop: u8, value: u32) -> Vec<u8> {
    let v = value.to_le_bytes();
    cmd(&[SET_PROPERTY, prop, 0x00, v[0], v[1], v[2]])
}

/// Colour buffer by LED id, then one write packet and as many stream packets as needed.
pub fn lighting_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let slots = VANGUARD
        .iter()
        .copied()
        .max()
        .map_or(0, |m| m as usize + 1)
        .max(VANGUARD.len());
    let mut data = DATA_TYPE.to_vec();
    data.resize(2 + slots * 3, 0);
    for (i, c) in VANGUARD.iter().zip(colors) {
        let at = 2 + *i as usize * 3;
        data[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    let first = (LEN - 8).min(data.len());
    let mut head = cmd(&[WRITE_ENDPOINT, LIGHTING_HANDLE]);
    head.extend((data.len() as u32).to_le_bytes());
    head.extend_from_slice(&data[..first]);
    let mut v = vec![head];
    for ch in data[first..].chunks(LEN - 4) {
        let mut p = cmd(&[STREAM_ENDPOINT, LIGHTING_HANDLE]);
        p.extend_from_slice(ch);
        v.push(p);
    }
    v
}

/// Every command is answered on the same endpoint; the answer is read and dropped.
fn send(io: &mut dyn Io, p: &[u8]) -> Result<(), String> {
    io.write(p, LEN)?;
    let _ = crate::hid::read_until(io, LEN, REPLY_MS, |_| true);
    Ok(())
}

struct Vanguard;

impl Proto for Vanguard {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        drain(io, LEN);
        send(io, &cmd(&[PING]))?;
        for h in HANDLES {
            send(io, &cmd(&[CLOSE_HANDLE, 0x01, h]))?;
        }
        send(io, &set_property(PROP_MODE, MODE_SOFTWARE))?;
        send(io, &set_property(PROP_BRIGHTNESS, BRIGHTNESS_MAX))?;
        send(
            io,
            &cmd(&[OPEN_ENDPOINT, LIGHTING_HANDLE, LIGHTING_CONTROLLER]),
        )
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in lighting_packets(colors) {
            send(io, &p)?;
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        for h in HANDLES {
            let _ = send(io, &cmd(&[CLOSE_HANDLE, 0x01, h]));
        }
        let _ = send(io, &set_property(PROP_MODE, MODE_HARDWARE));
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("corsair-vanguard"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD_RECT,
        [19, 6],
        VANGUARD_CELLS,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "corsair-vanguard", Vanguard).boxed())
}

pub static DETECTORS: &[Detector] = &[full(
    "Corsair Vanguard 96",
    0x1B1C,
    0x2B0D,
    2,
    0xFF42,
    0x0001,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(
            set_property(PROP_BRIGHTNESS, 1000),
            vec![0, 8, 1, 2, 0, 0xE8, 0x03, 0]
        );
        assert_eq!(
            set_property(PROP_MODE, MODE_HARDWARE),
            vec![0, 8, 1, 3, 0, 1, 0, 0]
        );
    }

    #[test]
    fn lighting_fits_one_write() {
        let p = lighting_packets(&vec![Color(1, 2, 3); VANGUARD.len()]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0][..4], [0, 8, 6, 0]);
        let n = u32::from_le_bytes([p[0][4], p[0][5], p[0][6], p[0][7]]) as usize;
        assert_eq!(p[0].len(), 8 + n);
        assert_eq!(p[0][8..10], [0x12, 0x00]);
    }
}
