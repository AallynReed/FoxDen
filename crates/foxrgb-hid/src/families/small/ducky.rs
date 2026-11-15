//! Ported from OpenRGB Controllers/DuckyKeyboardController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "ducky";
const TKL_PID: u16 = 0x0356;

const FULL_MAP: [&[u32]; 6] = [
    &[
        0, NA, 12, 18, 24, 30, NA, 42, 48, 54, 60, NA, 66, 72, 78, 84, 90, 96, 102, 108, 114, 120,
        126,
    ],
    &[
        1, 7, 13, 19, 25, 31, 37, 43, 49, 55, 61, NA, 67, 73, 85, NA, 91, 97, 103, 109, 115, 121,
        127,
    ],
    &[
        2, NA, 8, 14, 20, 26, NA, 32, 38, 44, 50, 56, 62, 68, 74, 86, 92, 98, 104, 110, 116, 122,
        128,
    ],
    &[
        3, NA, 9, 15, 21, 27, NA, 33, 39, 45, 51, 57, 63, 69, 75, 87, NA, NA, NA, 111, 117, 123, NA,
    ],
    &[
        4, 10, 16, 22, 28, 34, NA, 40, NA, 46, 52, 58, 64, 70, 82, NA, NA, 100, NA, 112, 118, 124,
        131,
    ],
    &[
        5, 11, 17, NA, NA, NA, NA, 41, NA, NA, NA, NA, 65, 77, 83, 89, 95, 101, 107, 113, NA, 125,
        NA,
    ],
];

const TKL_MAP: [&[u32]; 6] = [
    &[
        0, NA, 12, 18, 24, 30, NA, 42, 48, 54, 60, NA, 66, 72, 78, 84, 90, 96, 102,
    ],
    &[
        1, 7, 13, 19, 25, 31, 37, 43, 49, 55, 61, NA, 67, 73, 85, NA, 91, 97, 103,
    ],
    &[
        2, NA, 8, 14, 20, 26, NA, 32, 38, 44, 50, 56, 62, 68, 74, 86, 92, 98, 104,
    ],
    &[
        3, NA, 9, 15, 21, 27, NA, 33, 39, 45, 51, 57, 63, 69, 75, 87, NA, NA, NA,
    ],
    &[
        4, 10, 16, 22, 28, 34, NA, 40, NA, 46, 52, 58, 64, 70, 82, NA, NA, 100, NA,
    ],
    &[
        5, 11, 17, NA, NA, NA, NA, 41, NA, NA, NA, NA, 65, 77, 83, 89, 95, 101, 107,
    ],
];

fn packet(head: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

/// SendColors: start packet, eight data packets (40 then 60 bytes each), terminate packet.
pub fn colour_packets(colors: &[Color]) -> Vec<[u8; 65]> {
    let mut data = [0u8; 155 * 3];
    for (i, c) in colors.iter().enumerate() {
        data[i * 3..i * 3 + 3].copy_from_slice(&rgb(*c));
    }
    let mut out = vec![packet(&[
        0x56, 0x81, 0, 0, 0x01, 0, 0, 0, 0x08, 0, 0, 0, 0xAA, 0xAA, 0xAA, 0xAA,
    ])];
    let mut at = 0;
    for id in 0..8u8 {
        let mut p = packet(&[0x56, 0x83, id]);
        let start = if id == 0 {
            p[0x05] = 0x01;
            p[0x09] = 0x80;
            p[0x0A] = 0x01;
            p[0x0C] = 0xC1;
            p[0x11..0x15].copy_from_slice(&[0xFF; 4]);
            0x19
        } else {
            0x05
        };
        let n = 65 - start;
        p[start..].copy_from_slice(&data[at..at + n]);
        at += n;
        out.push(p);
    }
    out.push(packet(&[0x51, 0x28, 0, 0, 0xFF]));
    out
}

struct Ducky;

impl Proto for Ducky {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&packet(&[0x41, 0x01]))?;
        io.sleep(2);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in colour_packets(colors) {
            io.write(&p)?;
            io.sleep(2);
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (map, n): (&[&[u32]], usize) = if info.pid == TKL_PID {
        (&TKL_MAP, 108)
    } else {
        (&FULL_MAP, 132)
    };
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, map, n);
    boxed(api, info, FAMILY, dev, Ducky)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "Ducky Shine 7/Ducky One 2 RGB",
        0x04D9,
        0x0348,
        1,
        ANY,
        ANY,
        open,
    ),
    det("Ducky One 2 RGB TKL", 0x04D9, TKL_PID, 1, ANY, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SendInitialize from the constructor, then SendColors
    #[test]
    fn init_then_packets() {
        let mut a = vec![Color::BLACK; 132];
        a[0] = Color(1, 2, 3);
        a[14] = Color(4, 5, 6); // bytes 42..45: packet 1 offset 2
        let [first, _, _] = pushes(Ducky, 132, vec![], &a, &[]);
        assert_eq!(first[0], w(pad(&[0, 0x41, 0x01], 65)));
        assert_eq!(first[1], Op::Sleep(2));
        assert_eq!(first.len(), 2 + 10 * 2);
        assert_eq!(
            first[2],
            w(pad(
                &[
                    0, 0x56, 0x81, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 0xAA, 0xAA, 0xAA, 0xAA
                ],
                65
            ))
        );
        let mut p0 = pad(
            &[
                0, 0x56, 0x83, 0, 0, 1, 0, 0, 0, 0x80, 1, 0, 0xC1, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF,
                0xFF,
            ],
            65,
        );
        p0[0x19..0x1C].copy_from_slice(&[1, 2, 3]);
        assert_eq!(first[4], w(p0));
        let mut p1 = pad(&[0, 0x56, 0x83, 1], 65);
        p1[5 + 2..5 + 5].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[6], w(p1));
        assert_eq!(first[20], w(pad(&[0, 0x51, 0x28, 0, 0, 0xFF], 65)));
    }
}
