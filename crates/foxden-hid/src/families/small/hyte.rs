//! Ported from OpenRGB Controllers/HYTEKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::klm::{Klm, Layout, Op as KeyOp, TKL, k};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "hyte-keeb";
const SURROUND: usize = 63;

const VALUES: [u32; 89] = [
    0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58,
    63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93,
    94, 95, 97, 99, 105, 106, 107, 111, 115, 116, 117, 118, 119, 120, 121,
];

fn keyboard() -> Klm {
    let mut kb = Klm::new(Layout::Default, TKL, &VALUES, &[]);
    kb.change_keys(&[
        k(5, 4, 109, "Space", KeyOp::SwapOnly),
        k(5, 5, 110, "Space", KeyOp::SwapOnly),
        k(5, 7, 112, "Space", KeyOp::SwapOnly),
        k(5, 8, 113, "Space", KeyOp::SwapOnly),
        k(0, 0, 77, "Media Stop", KeyOp::InsertRow),
        k(0, 1, 78, "Media Previous", KeyOp::SwapOnly),
        k(0, 2, 79, "Media Play/Pause", KeyOp::SwapOnly),
        k(0, 3, 98, "Media Next", KeyOp::SwapOnly),
        k(0, 4, 100, "Media Mute", KeyOp::SwapOnly),
    ]);
    kb
}

/// LEDStreaming: a feature report naming the zone, then the colours as packed RGB in 64-byte writes.
pub fn streaming(zone: u8, colors: &[Color], packets: usize) -> Vec<Vec<u8>> {
    let mut out = vec![vec![0, 0x04, zone, 0, 0, 0, 0, 0, 0]];
    let bytes: Vec<u8> = colors.iter().flat_map(|c| rgb(*c)).collect();
    for p in 0..packets {
        let mut b = vec![0u8; 65];
        for i in 0..64 {
            b[1 + i] = bytes.get(p * 64 + i).copied().unwrap_or(0);
        }
        out.push(b);
    }
    out
}

struct Hyte {
    values: Vec<u32>,
}

impl Proto for Hyte {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let keys = self.values.len();
        let mut buf = [Color::BLACK; 128];
        for (&v, &c) in self.values.iter().zip(colors) {
            buf[v as usize] = c;
        }
        let zones = [
            streaming(0xF0, &buf, 6),
            streaming(0xF1, &colors[keys..keys + SURROUND], 3),
        ];
        for zone in zones {
            io.feature(&zone[0])?;
            zone[1..].iter().try_for_each(|p| io.write(p))?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let kb = keyboard();
    let mut dev = kb.device(info, FAMILY, d);
    for l in &mut dev.leds {
        l[1] *= 0.85;
    }
    dev.leds
        .extend((0..SURROUND).map(|i| [(i as f32 + 0.5) / SURROUND as f32, 0.95]));
    boxed(
        api,
        info,
        FAMILY,
        dev,
        Hyte {
            values: kb.values(),
        },
    )
}

pub static DETECTORS: &[Detector] = &[det(
    "HYTE Keeb TKL",
    0x3402,
    0x0300,
    ANY,
    0xFF11,
    0xF0,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout_counts() {
        let kb = keyboard();
        assert_eq!(kb.keys.len(), 89 + 4 + 5);
        assert_eq!(kb.keys.iter().filter(|k| k.row == 0).count(), 5);
    }

    // LEDStreaming_Keyboard then LEDStreaming_Surround
    #[test]
    fn keyboard_then_surround() {
        let kb = keyboard();
        let keys = kb.keys.len();
        let n = keys + SURROUND;
        let mut a = vec![Color::BLACK; n];
        let esc = kb.keys.iter().position(|k| k.name == "Escape").unwrap();
        a[esc] = Color(1, 2, 3); // value 0
        a[keys] = Color(7, 8, 9);
        a[n - 1] = Color(4, 5, 6);
        let [first, _, _] = pushes(
            Hyte {
                values: kb.values(),
            },
            n,
            vec![],
            &a,
            &[],
        );
        assert_eq!(first.len(), 1 + 6 + 1 + 3);
        assert_eq!(first[0], Op::F(vec![0, 4, 0xF0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(first[1], w(pad(&[0, 1, 2, 3], 65)));
        assert_eq!(first[7], Op::F(vec![0, 4, 0xF1, 0, 0, 0, 0, 0, 0]));
        assert_eq!(first[8], w(pad(&[0, 7, 8, 9], 65)));
        let mut last = vec![0u8; 65];
        last[1 + 62 * 3 - 128..1 + 63 * 3 - 128].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[10], w(last));
    }
}
