//! HyperX Pulsefire FPS Pro (0x1729), Haste Wireless and Haste 2 mice (protocol learned from
//! SignalRGB's HyperX_Pulsefire_FPS_Pro.js, HyperX_PulseFire_Haste_Wireless_Mouse.js and
//! HyperX_Pulsefire_Haste_2_Mouse.js).

use super::{MOUSE, boxed, det, placed};
use crate::hid::{Io, Proto, drain, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-pulsefire";
const KINGSTON: u16 = 0x0951;
const HP: u16 = 0x03F0;

#[derive(Clone, Copy)]
enum Kind {
    FpsPro,
    HasteWireless,
    Haste2,
}

pub static DETECTORS: &[Detector] = &[
    det(
        "HyperX Pulsefire FPS Pro",
        KINGSTON,
        0x1729,
        Some(2),
        None,
        None,
        open_fps,
    ),
    det(
        "HyperX Pulsefire Haste Wireless",
        HP,
        0x048E,
        Some(2),
        Some(0xFF00),
        Some(0x01),
        open_haste,
    ),
    det(
        "HyperX Pulsefire Haste Wireless",
        HP,
        0x028E,
        Some(2),
        Some(0xFF00),
        Some(0x01),
        open_haste,
    ),
    det(
        "HyperX Pulsefire Haste 2",
        HP,
        0x0B97,
        Some(2),
        Some(0xFF00),
        Some(0x01),
        open_haste2,
    ),
    det(
        "HyperX Pulsefire Haste 2",
        HP,
        0x0D97,
        Some(2),
        Some(0xFF00),
        Some(0x01),
        open_haste2,
    ),
    det(
        "HyperX Pulsefire Haste 2",
        HP,
        0x0F98,
        Some(2),
        Some(0xFF00),
        Some(0x01),
        open_haste2,
    ),
    det(
        "HyperX Pulsefire Haste 2 Mini",
        HP,
        0x0DA0,
        Some(2),
        Some(0xFF90),
        Some(0xFF00),
        open_haste2,
    ),
    det(
        "HyperX Pulsefire Haste 2 Mini",
        HP,
        0x0BA0,
        Some(2),
        Some(0xFF90),
        Some(0xFF00),
        open_haste2,
    ),
];

/// `sendColors` of the FPS Pro (0xD8, six bytes) and Haste Wireless (0xD2, eight): RGB per LED, then 0x64.
pub fn d_report(cmd: u8, count: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0x00, cmd, 0x00, 0x00, count];
    for &c in colors.iter().take(2) {
        b.extend(rgb(c));
    }
    b.resize(11, 0);
    b.push(0x64);
    b
}

/// Haste 2: 0x44 0x01 arms direct lighting before every 0x44 0x02 colour frame.
pub fn haste2(c: Color) -> [Vec<u8>; 2] {
    [
        vec![0x44, 0x01, 0x01],
        vec![0x44, 0x02, 0x00, 0x00, c.0, c.1, c.2],
    ]
}

struct Pulsefire(Kind);

impl Proto for Pulsefire {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if let Kind::HasteWireless = self.0 {
            io.write(&[0x00, 0x46], 64)?;
            io.write(&[0x00, 0x57, 0x12, 0x01], 64)?;
            io.write(&[0x00, 0x51], 64)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        match self.0 {
            Kind::FpsPro => io.write(&d_report(0xD8, 0x06, c), 65),
            Kind::HasteWireless => io.write(&d_report(0xD2, 0x08, c), 64),
            Kind::Haste2 => {
                drain(io, 64);
                for p in haste2(c[0]) {
                    io.write(&p, 65)?;
                }
                Ok(())
            }
        }
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); 2]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector, kind: Kind) -> Result<Box<dyn Backend>, String> {
    let pos: &[[u8; 2]] = match kind {
        Kind::FpsPro => &[[1, 0], [1, 2]],
        Kind::HasteWireless => &[[1, 0], [1, 1]],
        Kind::Haste2 => &[[1, 0]],
    };
    let dev = placed(info, FAMILY, d.name, DeviceKind::Mouse, MOUSE, [3, 3], pos);
    boxed(api, info, "hyperx pulsefire", dev, Pulsefire(kind))
}

fn open_fps(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Kind::FpsPro)
}

fn open_haste(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Kind::HasteWireless)
}

fn open_haste2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Kind::Haste2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d_reports() {
        let c = [Color(1, 2, 3), Color(4, 5, 6)];
        assert_eq!(
            d_report(0xD8, 6, &c),
            [0, 0xD8, 0, 0, 6, 1, 2, 3, 4, 5, 6, 0x64]
        );
        assert_eq!(d_report(0xD2, 8, &c)[1], 0xD2);
    }

    #[test]
    fn haste2_arms_first() {
        let [a, b] = haste2(Color(7, 8, 9));
        assert_eq!(a, [0x44, 1, 1]);
        assert_eq!(b, [0x44, 2, 0, 0, 7, 8, 9]);
    }
}
