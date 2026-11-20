//! Ported from OpenRGB Controllers/PatriotViperMouseController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "patriot-viper";
const LEDS: usize = 7;

pub const INIT: [u8; 64] = [
    0x01, 0x00, 0x12, 0x12, 0x00, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0x50, 0xDE, 0x8D, 0x77, 0x09, 0xDF, 0x8D, 0x77, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x30, 0x58, 0x7C, 0x77, 0x78, 0x81, 0x43, 0x00, 0x30, 0x58, 0x7C, 0x77,
    0x8C, 0x5D, 0x9B, 0x77, 0x00, 0x00, 0x3D, 0x00, 0x98, 0xF5, 0x19, 0x08, 0x00, 0x00, 0x00, 0xEE,
];

pub fn led_report(led: u8, c: Color) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..7].copy_from_slice(&[0x01, 0x13, led, 0xFF, c.0, c.1, c.2]);
    let x = b[..63].iter().fold(0u8, |a, &v| a ^ v);
    b[63] = if x % 2 == 0 { x.wrapping_add(1) } else { x - 1 };
    b
}

struct Viper;

impl Proto for Viper {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&INIT)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        (0..LEDS).try_for_each(|i| io.feature(&led_report(i as u8, colors[i])))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Mouse,
        3,
        4,
        &[(0, 1), (0, 2), (0, 3), (2, 1), (2, 2), (2, 3), (1, 0)],
    );
    boxed(api, info, FAMILY, dev, Viper)
}

pub static DETECTORS: &[Detector] = &[det(
    "Patriot Viper V550",
    0x0C45,
    0x7E18,
    2,
    0xFF18,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // PatriotViperMouseController constructor, then SetRGB
    #[test]
    fn init_then_leds() {
        let a = [Color(0, 0, 0); LEDS];
        let mut b = a;
        b[6] = Color(0x10, 0x20, 0x31);
        let [first, _, next] = pushes(Viper, LEDS, vec![], &a, &b);
        assert_eq!(first.len(), 8);
        assert_eq!(first[0], Op::F(INIT.to_vec()));
        // 0x01 ^ 0x13 ^ 0xFF = 0xED, odd, so 0xEC
        assert_eq!(
            first[1],
            Op::F(
                pad(&[0x01, 0x13, 0, 0xFF], 63)
                    .into_iter()
                    .chain([0xEC])
                    .collect()
            )
        );
        // 0xED ^ 0x01 = 0xEC, even, so 0xED
        assert_eq!(
            first[2],
            Op::F(
                pad(&[0x01, 0x13, 1, 0xFF], 63)
                    .into_iter()
                    .chain([0xED])
                    .collect()
            )
        );
        // 0xED ^ 6 ^ 0x10 ^ 0x20 ^ 0x31 = 0xEA, even, so 0xEB
        let mut e = pad(&[0x01, 0x13, 6, 0xFF, 0x10, 0x20, 0x31], 64);
        e[63] = 0xEB;
        assert_eq!(next[6], Op::F(e));
    }
}
