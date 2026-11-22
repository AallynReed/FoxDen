// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-origins-core";
const ABNT2: u8 = 0x10;

// KeyboardLayoutManager TKL output for hyperx_core_layout: LED index per cell, then each LED's value
const MAP_ANSI: [&[u32]; 6] = [
    &[0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    &[
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    ],
    &[
        33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    ],
    &[
        50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, NA, 62, NA, NA, NA,
    ],
    &[
        63, NA, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, NA, 74, NA, 75, NA,
    ],
    &[
        76, 77, 78, NA, NA, NA, 79, NA, NA, NA, 80, 81, 82, 83, 84, 85, 86,
    ],
];
const VALUES_ANSI: [u32; 87] = [
    0, 1, 2, 3, 4, 5, 6, 7, 48, 49, 50, 51, 52, 53, 54, 55, 8, 9, 10, 11, 12, 13, 14, 15, 16, 56,
    57, 58, 59, 60, 61, 62, 63, 17, 18, 19, 20, 21, 22, 23, 24, 64, 65, 66, 67, 68, 69, 70, 71, 72,
    25, 26, 27, 28, 29, 30, 31, 32, 73, 74, 75, 76, 78, 33, 35, 36, 37, 38, 39, 40, 79, 80, 81, 82,
    84, 85, 41, 42, 43, 45, 86, 87, 88, 89, 90, 91, 92,
];

const MAP_ABNT2: [&[u32]; 6] = [
    &[0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    &[
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    ],
    &[
        33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, NA, 46, 47, 48,
    ],
    &[
        49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, NA, NA, NA,
    ],
    &[
        63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, NA, 77, NA,
    ],
    &[
        78, 79, 80, NA, NA, NA, 81, NA, NA, NA, 82, 83, 84, 85, 86, 87, 88,
    ],
];
const VALUES_ABNT2: [u32; 89] = [
    0, 1, 2, 3, 4, 5, 6, 7, 48, 49, 50, 51, 52, 53, 54, 55, 8, 9, 10, 11, 12, 13, 14, 15, 16, 56,
    57, 58, 59, 60, 61, 62, 63, 17, 18, 19, 20, 21, 22, 23, 24, 64, 65, 66, 67, 68, 70, 71, 72, 25,
    26, 27, 28, 29, 30, 31, 32, 73, 74, 75, 76, 77, 78, 33, 34, 35, 36, 37, 38, 39, 40, 79, 80, 81,
    82, 83, 84, 85, 41, 42, 43, 45, 86, 87, 88, 89, 90, 91, 92,
];

/// GetVariant: byte 56 of the reply to a 0x10 query, 0 if none.
fn variant(io: &mut dyn Io) -> Result<u8, String> {
    io.write(&pad65(&[0x00, 0x10]))?;
    let mut buf = [0u8; 65];
    // the C++ read blocks; bounded here so a silent keyboard cannot hang detection
    let n = io.read(&mut buf, 1000)?;
    Ok(if n > 0 { buf[56] } else { 0 })
}

fn pad65(head: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[..head.len()].copy_from_slice(head);
    b
}

struct Core {
    values: &'static [u32],
}

impl Proto for Core {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&pad65(&[0x00, 0xA7, 0x00, 0x00, 0x01, 0xFF]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut buf = [0u8; 380];
        for (c, &v) in colors.iter().zip(self.values).take(94) {
            let v = v as usize;
            let pos = (v / 12) * 48 + ((v / 6) & 1) * 8 + v % 6;
            buf[pos] = c.1;
            buf[pos + 16] = c.0;
            buf[pos + 32] = c.2;
        }
        for (seq, chunk) in buf.chunks(60).enumerate() {
            let mut p = [0u8; 65];
            p[1] = 0xA2;
            p[2] = seq as u8;
            p[4] = chunk.len() as u8;
            p[5..5 + chunk.len()].copy_from_slice(chunk);
            io.write(&p[..5 + chunk.len()])?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let (map, values): (&[&[u32]], &'static [u32]) = if variant(io.as_mut())? == ABNT2 {
        (&MAP_ABNT2, &VALUES_ABNT2)
    } else {
        (&MAP_ANSI, &VALUES_ANSI)
    };
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, map, values.len());
    Ok(Box::new(Driver::new(io, dev, FAMILY, Core { values })))
}

pub static DETECTORS: &[Detector] = &[
    det(
        "HyperX Alloy Origins Core",
        0x0951,
        0x16E6,
        2,
        ANY,
        ANY,
        open,
    ),
    det(
        "HyperX Alloy Origins Core (HP)",
        0x03F0,
        0x098F,
        2,
        ANY,
        ANY,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layouts_match_klm() {
        for (map, n) in [(&MAP_ANSI, 87u32), (&MAP_ABNT2, 89)] {
            let mut seen: Vec<u32> = map.iter().flat_map(|r| r.iter().copied()).collect();
            seen.retain(|&v| v != NA);
            seen.sort();
            assert_eq!(seen, (0..n).collect::<Vec<_>>());
        }
    }

    #[test]
    fn reads_variant() {
        let mut reply = vec![0u8; 65];
        reply[56] = 0x10;
        let mut rec = Rec::with(vec![reply]);
        assert_eq!(variant(&mut rec), Ok(ABNT2));
        assert_eq!(rec.take(), vec![w(pad(&[0, 0x10], 65)), Op::Read]);
        assert_eq!(variant(&mut Rec::default()), Ok(0));
    }

    #[test]
    fn brightness_then_buffer() {
        let mut a = vec![Color::BLACK; 87];
        a[0] = Color(1, 2, 3); // value 0: G at 0, R at 16, B at 32
        a[86] = Color(4, 5, 6); // value 92: G at 346, R at 362, B at 378
        let core = Core {
            values: &VALUES_ANSI,
        };
        let [first, _, _] = pushes(core, 87, vec![], &a, &[]);
        assert_eq!(first.len(), 8);
        assert_eq!(first[0], w(pad(&[0, 0xA7, 0, 0, 1, 0xFF], 65)));
        let mut p0 = pad(&[0, 0xA2, 0, 0, 60], 65);
        p0[5] = 2;
        p0[5 + 16] = 1;
        p0[5 + 32] = 3;
        assert_eq!(first[1], w(p0));
        for seq in 1..5u8 {
            assert_eq!(first[1 + seq as usize], w(pad(&[0, 0xA2, seq, 0, 60], 65)));
        }
        let mut p5 = pad(&[0, 0xA2, 5, 0, 60], 65);
        p5[5 + 46] = 5;
        assert_eq!(first[6], w(p5));
        let mut p6 = pad(&[0, 0xA2, 6, 0, 20], 25);
        p6[5 + 2] = 4;
        p6[5 + 18] = 6;
        assert_eq!(first[7], w(p6));
    }
}
