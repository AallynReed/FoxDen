// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "skydimo";
const LEDS: usize = 49;
const DATA: usize = 60;

pub fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut crc, &b| {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// SetLEDs: GRB frame in 60-byte chunks, each sealed with CRC-8, then the end packet.
pub fn packets(colors: &[Color]) -> Vec<[u8; 64]> {
    let frame: Vec<u8> = colors
        .iter()
        .take(LEDS)
        .flat_map(|c| [c.1, c.0, c.2])
        .collect();
    let mut out: Vec<[u8; 64]> = frame
        .chunks(DATA)
        .enumerate()
        .map(|(n, chunk)| {
            let mut p = [0u8; 64];
            p[0] = 0x01;
            p[1] = (n * DATA / 3) as u8;
            p[3..3 + chunk.len()].copy_from_slice(chunk);
            p[63] = crc8(&p[..63]);
            p
        })
        .collect();
    let mut end = [0u8; 64];
    end[..4].copy_from_slice(&[0x01, 0xFF, 0xFF, LEDS as u8]);
    end[60] = 0x1E;
    out.push(end);
    out
}

struct Skydimo;

impl Proto for Skydimo {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        packets(colors).iter().try_for_each(|p| io.write(p))
    }
}

/// skydimo_hid_led_positions: a 7 x 7 snake, column by column.
fn positions() -> Vec<(u32, u32)> {
    (0..7u32)
        .flat_map(|c| (0..7u32).map(move |r| (c, if c % 2 == 0 { 6 - r } else { r })))
        .collect()
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(info, FAMILY, d, DeviceKind::Other, 7, 7, &positions());
    boxed(api, info, FAMILY, dev, Skydimo)
}

pub static DETECTORS: &[Detector] = &[det("Skydimo SK0902", 0x1A86, 0xE316, 0, 0xFF00, 0x01, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // CRC-8 with polynomial 0x07 and init 0: the standard check value
    #[test]
    fn crc_reference() {
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn snake_positions() {
        let p = positions();
        assert_eq!(
            p[..8],
            [
                (0, 6),
                (0, 5),
                (0, 4),
                (0, 3),
                (0, 2),
                (0, 1),
                (0, 0),
                (1, 0)
            ]
        );
        assert_eq!(p[48], (6, 0));
    }

    // SkydimoHIDController::SetLEDs
    #[test]
    fn three_chunks_and_end() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[48] = Color(4, 5, 6);
        let [first, _, _] = pushes(Skydimo, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 4);
        let mut p0 = pad(&[1, 0, 0, 2, 1, 3], 64);
        p0[63] = crc8(&p0[..63]);
        assert_eq!(first[0], w(p0));
        let Op::W(p2) = &first[2] else { panic!() };
        assert_eq!(p2[..3], [1, 40, 0]);
        assert_eq!(p2[3 + 24..3 + 27], [5, 4, 6]);
        assert_eq!(p2[3 + 27..63], [0; 33]);
        let mut end = pad(&[1, 0xFF, 0xFF, 49], 64);
        end[60] = 0x1E;
        assert_eq!(first[3], w(end));
    }
}
