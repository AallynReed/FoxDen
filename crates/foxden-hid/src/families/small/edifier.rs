// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "edifier-halo";
const AMBIENT: u8 = 0x6B;
const SCREEN: u8 = 0xEF;
const STATIC: u8 = 3;

pub fn packet(command: u8, payload: &[u8]) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[..6].copy_from_slice(&[
        0x2E,
        0xAA,
        0xEC,
        command,
        (payload.len() >> 8) as u8,
        payload.len() as u8,
    ]);
    p[6..6 + payload.len()].copy_from_slice(payload);
    let sum: u32 = p[1..6 + payload.len()].iter().map(|&b| b as u32).sum();
    p[6 + payload.len()] = sum as u8;
    p
}

pub fn ambient_power(on: bool) -> [u8; 64] {
    packet(AMBIENT, &[0, 0, 0, 0, on as u8, 0xFF, 0xFF])
}

/// SetAmbientLight; brightness 1..3 maps to 0x14, 0x28, 0x3C and speed is clamped to 1..10.
pub fn ambient(effect: u8, c: Color, brightness: u8, speed: u8) -> [u8; 64] {
    let raw = match brightness {
        1 => 0x14,
        2 => 0x28,
        _ => 0x3C,
    };
    packet(
        AMBIENT,
        &[0x13, effect, c.0, c.1, c.2, raw, speed.clamp(1, 10)],
    )
}

pub fn screen_color(c: Color) -> [u8; 64] {
    packet(SCREEN, &[0x03, c.0, c.1, c.2])
}

/// The light effects in Static, its default mode (no per-LED mode exists), then the pixel display colour.
struct Halo;

impl Proto for Halo {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&ambient_power(true))?;
        io.write(&ambient(STATIC, colors[0], 3, 0))?;
        io.write(&screen_color(colors[1]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Strip, 2);
    boxed(api, info, FAMILY, dev, Halo)
}

pub static DETECTORS: &[Detector] = &[det(
    "Edifier Halo Pixelbar",
    0x2D99,
    0xA106,
    4,
    0xFF14,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetAmbientLightPower, SetAmbientLight (Static) and SetPixelScreenColor via SendEdifierPacket
    #[test]
    fn three_packets() {
        let [first, _, _] = pushes(Halo, 2, vec![], &[Color(1, 2, 3), Color(4, 5, 6)], &[]);
        let sum = |b: &[u8]| b.iter().map(|&v| v as u32).sum::<u32>() as u8;
        // 0xAA + 0xEC + 0x6B + 7 + 1 + 0xFF + 0xFF = 0x407
        let p = pad(
            &[
                0x2E, 0xAA, 0xEC, 0x6B, 0, 7, 0, 0, 0, 0, 1, 0xFF, 0xFF, 0x07,
            ],
            64,
        );
        assert_eq!(first[0], w(p));
        let mut p = pad(
            &[0x2E, 0xAA, 0xEC, 0x6B, 0, 7, 0x13, 3, 1, 2, 3, 0x3C, 1],
            64,
        );
        p[13] = sum(&p[1..13]);
        assert_eq!(first[1], w(p));
        let mut p = pad(&[0x2E, 0xAA, 0xEC, 0xEF, 0, 4, 3, 4, 5, 6], 64);
        p[10] = sum(&p[1..10]);
        assert_eq!(first[2], w(p));
        assert_eq!(first.len(), 3);
    }
}
