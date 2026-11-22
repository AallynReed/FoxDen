// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "valkyrie";

const PRO_MATRIX: [[u32; 22]; 6] = [
    [
        0, NA, 1, 2, 3, 4, NA, 5, 6, 7, 8, NA, 9, 10, 11, 12, NA, 13, NA, NA, NA, NA,
    ],
    [
        14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, NA, NA, NA, 28, 29, 30, 31, 32,
    ],
    [
        33, NA, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, NA, NA, 47, 48, 49, 50, 51,
    ],
    [
        52, NA, NA, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, NA, 63, 64, NA, 65, 66, 67, 68, NA,
    ],
    [
        69, NA, NA, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, NA, NA, 81, NA, 82, 83, 84, 85,
    ],
    [
        86, 87, 88, NA, NA, NA, NA, 89, NA, NA, NA, NA, 90, 91, 92, 93, 94, 95, NA, 96, 97, NA,
    ],
];

const NORMAL_MATRIX: [[u32; 22]; 6] = [
    [
        0, NA, 1, 2, 3, 4, NA, 5, 6, 7, 8, NA, 9, 10, 11, 12, NA, 13, 14, 15, 16, 17,
    ],
    [
        18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, NA, NA, NA, 32, 33, 34, 35, 36,
    ],
    [
        37, NA, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, NA, NA, 51, 52, 53, 54, 55,
    ],
    [
        56, NA, NA, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, NA, 67, 68, NA, 69, 70, 71, 72, NA,
    ],
    [
        73, NA, NA, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, NA, NA, 85, NA, 86, 87, 88, 89,
    ],
    [
        90, 91, 92, NA, NA, NA, NA, 93, NA, NA, NA, NA, 94, 95, 96, 97, 98, 99, NA, 100, 101, NA,
    ],
];

const KEY_CODE_99: [u8; 98] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x77, 0x13, 0x14,
    0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x67, 0x74, 0x20, 0x21, 0x22,
    0x7A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x43, 0x76,
    0x32, 0x33, 0x34, 0x7B, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42,
    0x55, 0x79, 0x44, 0x45, 0x46, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53,
    0x54, 0x65, 0x56, 0x57, 0x58, 0x6A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x62, 0x63, 0x64, 0x66,
    0x68, 0x69,
];

const KEY_CODE_103: [u8; 102] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x77, 0x70, 0x73,
    0x75, 0x78, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x67,
    0x74, 0x20, 0x21, 0x22, 0x7A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F,
    0x30, 0x31, 0x43, 0x76, 0x32, 0x33, 0x34, 0x7B, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E,
    0x3F, 0x40, 0x41, 0x42, 0x55, 0x79, 0x44, 0x45, 0x46, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F,
    0x50, 0x51, 0x52, 0x53, 0x54, 0x65, 0x56, 0x57, 0x58, 0x6A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60,
    0x62, 0x63, 0x64, 0x66, 0x68, 0x69,
];

/// SendColors' seven data reports: key code then RGB, sixteen keys a report.
pub fn colour_reports(codes: &[u8], colors: &[Color]) -> Vec<[u8; 65]> {
    let mut data: Vec<u8> = Vec::new();
    for (&k, c) in codes.iter().zip(colors) {
        data.extend([k, c.0, c.1, c.2]);
    }
    (0..7)
        .map(|i| {
            let mut b = [0u8; 65];
            let chunk = &data[(i * 64).min(data.len())..((i + 1) * 64).min(data.len())];
            b[1..1 + chunk.len()].copy_from_slice(chunk);
            b
        })
        .collect()
}

fn feature(io: &mut dyn Io, head: &[u8]) -> Result<(), String> {
    let mut b = [0u8; 65];
    b[..head.len()].copy_from_slice(head);
    io.feature(&b)?;
    io.sleep(1);
    Ok(())
}

fn ack(io: &mut dyn Io) -> Result<(), String> {
    let mut b = [0u8; 65];
    io.get_feature(&mut b)?;
    io.sleep(1);
    Ok(())
}

struct Valkyrie {
    codes: &'static [u8],
}

impl Proto for Valkyrie {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        feature(io, &[0, 0x04, 0x20, 0, 0, 0, 0, 0, 0, 0x08])?;
        ack(io)?;
        for r in colour_reports(self.codes, colors) {
            io.feature(&r)?;
            io.sleep(1);
        }
        feature(io, &[])?;
        feature(io, &[0, 0x04, 0x02])?;
        ack(io)?;
        io.sleep(33);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (matrix, codes): (&[[u32; 22]; 6], &'static [u8]) = if info.interface == 3 {
        (&PRO_MATRIX, &KEY_CODE_99)
    } else {
        (&NORMAL_MATRIX, &KEY_CODE_103)
    };
    let map: Vec<&[u32]> = matrix.iter().map(|r| &r[..]).collect();
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &map, codes.len());
    boxed(api, info, FAMILY, dev, Valkyrie { codes })
}

pub static DETECTORS: &[Detector] = &[
    det("Valkyrie VK99 Pro", 0x05AC, 0x024F, 3, 0xFF13, 0x0001, open),
    det("Valkyrie VK99", 0x05AC, 0x024F, 2, 0xFF13, 0x0001, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // ValkyrieKeyboardController::SendColors on the Pro (interface 3)
    #[test]
    fn pro_sequence() {
        let mut a = vec![Color::BLACK; 98];
        a[0] = Color(1, 2, 3);
        a[97] = Color(4, 5, 6);
        let [first, _, _] = pushes(
            Valkyrie {
                codes: &KEY_CODE_99,
            },
            98,
            vec![],
            &a,
            &[],
        );
        assert_eq!(first.len(), 4 + 14 + 7);
        assert_eq!(first[0], Op::F(pad(&[0, 4, 0x20, 0, 0, 0, 0, 0, 0, 8], 65)));
        assert_eq!(first[1], Op::Sleep(1));
        assert_eq!(first[2], Op::GetF(0));
        let Op::F(r0) = &first[4] else { panic!() };
        assert_eq!(r0[..13], [0, 0x01, 1, 2, 3, 0x02, 0, 0, 0, 0x03, 0, 0, 0]);
        assert_eq!(r0[61..], [0x14, 0, 0, 0]);
        // the seventh report holds keys 96 and 97 only
        assert_eq!(
            first[16],
            Op::F(pad(&[0, 0x68, 0, 0, 0, 0x69, 4, 5, 6], 65))
        );
        assert_eq!(first[18], Op::F(vec![0; 65]));
        assert_eq!(first[20], Op::F(pad(&[0, 4, 2], 65)));
        assert_eq!(first[22], Op::GetF(0));
        assert_eq!(first[24], Op::Sleep(33));
    }

    #[test]
    fn normal_has_six_in_the_last_report() {
        let r = colour_reports(&KEY_CODE_103, &[Color(9, 9, 9); 102]);
        assert_eq!(r[6][1..25].iter().filter(|&&b| b == 9).count(), 18);
        assert_eq!(r[6][25..], [0; 40]);
    }
}
