//! Ported from OpenRGB Controllers/HyperXKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::hyperx_alloy_elite::{MATRIX, planes};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hyperx-alloy-fps";
const LEDS: usize = 106;

struct Fps;

impl Proto for Fps {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let [r, g, b] = planes(colors);
        io.feature(&r)?;
        io.sleep(10);
        io.feature(&g)?;
        io.sleep(10);
        io.feature(&b)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(50)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, &MATRIX, LEDS);
    boxed(api, info, FAMILY, dev, Fps)
}

pub static DETECTORS: &[Detector] = &[det(
    "HyperX Alloy FPS RGB",
    0x0951,
    0x16DC,
    2,
    0xFF01,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn three_planes() {
        let mut seen: Vec<u32> = MATRIX.iter().flat_map(|r| r.iter().copied()).collect();
        seen.retain(|&v| v != NA);
        seen.sort();
        assert_eq!(seen, (0..LEDS as u32).collect::<Vec<_>>());

        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[22] = Color(4, 5, 6); // key byte 0x1E
        a[23] = Color(7, 8, 9); // key byte 0x20
        let [first, _, _] = pushes(Fps, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), 5);
        let p = |ch: u8, v: [u8; 3]| {
            let mut p = pad(&[0x07, 0x16, ch, 0xA0], 264);
            p[0x08] = v[0];
            p[0x1E] = v[1];
            p[0x20] = v[2];
            Op::F(p)
        };
        assert_eq!(first[0], p(1, [1, 4, 7]));
        assert_eq!(first[1], Op::Sleep(10));
        assert_eq!(first[2], p(2, [2, 5, 8]));
        assert_eq!(first[3], Op::Sleep(10));
        assert_eq!(first[4], p(3, [3, 6, 9]));
    }
}
