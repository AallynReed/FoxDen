//! Ported from OpenRGB Controllers/WushiController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "wushi-l50";
const STATIC: u8 = 1;
const SPEED_SLOWEST: u8 = 1;
const BRIGHTNESS_HIGH: u8 = 2;

/// setMode on Windows: report 0xCC, then effect, speed, brightness and four zone colours.
pub fn set_mode(colors: &[Color]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[..5].copy_from_slice(&[0xCC, 0x16, STATIC, SPEED_SLOWEST, BRIGHTNESS_HIGH]);
    for (z, c) in colors.iter().take(4).enumerate() {
        b[5 + 3 * z..8 + 3 * z].copy_from_slice(&rgb(*c));
    }
    b
}

struct Wushi;

impl Proto for Wushi {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&set_mode(colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 4);
    boxed(api, info, FAMILY, dev, Wushi)
}

pub static DETECTORS: &[Detector] = &[det(
    "JSAUX RGB Docking Station",
    0x306F,
    0x1234,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // WushiL50USBController::setMode with the Direct state
    #[test]
    fn direct_state() {
        let a: Vec<Color> = (0..4).map(|i| Color(i, 10 + i, 20 + i)).collect();
        let [first, _, _] = pushes(Wushi, 4, vec![], &a, &[]);
        assert_eq!(
            first,
            vec![Op::F(pad(
                &[
                    0xCC, 0x16, 1, 1, 2, 0, 10, 20, 1, 11, 21, 2, 12, 22, 3, 13, 23, 0, 0
                ],
                65
            ))]
        );
    }
}
