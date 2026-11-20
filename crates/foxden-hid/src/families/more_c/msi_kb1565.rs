//! Ported from OpenRGB Controllers/MSIKeyboardController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "msi-kb1565";
const ZONES: [u8; 4] = [1, 2, 4, 8];

/// SetMode for the Static mode of one zone: speed 1200 (0x04B0), two key frames.
pub fn steady(zone: u8, c: Color) -> [[u8; 64]; 2] {
    let mut select = [0u8; 64];
    select[..3].copy_from_slice(&[0x02, 0x01, zone]);
    let mut data = [0u8; 64];
    data[..10].copy_from_slice(&[0x02, 0x02, 0x01, 0xB0, 0x04, 0, 0, 0x0F, 0x01, 0]);
    let [r, g, b] = rgb(c);
    data[10..18].copy_from_slice(&[0, r, g, b, 100, r, g, b]);
    [select, data]
}

struct Kb1565;

impl Proto for Kb1565 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (&zone, &c) in ZONES.iter().zip(colors) {
            for p in steady(zone, c) {
                io.feature(&p)?;
            }
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Keyboard, ZONES.len());
    boxed(api, info, FAMILY, dev, Kb1565)
}

pub static DETECTORS: &[Detector] = &[det(
    "MSI Keyboard MS_1565",
    0x1462,
    0x1601,
    ANY,
    0x00FF,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // "Static zone 1..4" modes, one per LED
    #[test]
    fn zone_packets() {
        let mut a = vec![Color::BLACK; 4];
        a[2] = Color(1, 2, 3);
        let [first, _, _] = pushes(Kb1565, 4, vec![], &a, &[]);
        assert_eq!(first.len(), 8);
        assert_eq!(first[0], Op::F(pad(&[2, 1, 1], 64)));
        assert_eq!(first[4], Op::F(pad(&[2, 1, 4], 64)));
        assert_eq!(first[6], Op::F(pad(&[2, 1, 8], 64)));
        assert_eq!(
            first[5],
            Op::F(pad(
                &[
                    2, 2, 1, 0xB0, 4, 0, 0, 0x0F, 1, 0, 0, 1, 2, 3, 0x64, 1, 2, 3
                ],
                64
            ))
        );
        assert_eq!(
            first[1],
            Op::F(pad(
                &[2, 2, 1, 0xB0, 4, 0, 0, 0x0F, 1, 0, 0, 0, 0, 0, 0x64],
                64
            ))
        );
    }
}
