//! Ported from OpenRGB Controllers/ThingMController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "thingm-blink";

/// Fade-to-colour command; direct mode sends speed 0.
pub fn update(led: u8, c: Color, speed: u16) -> [u8; 9] {
    let [hi, lo] = speed.to_be_bytes();
    [0x01, 0x63, c.0, c.1, c.2, hi, lo, led, 0x00]
}

struct Blink;

impl Proto for Blink {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&update(1, colors[0], 0))?;
        io.feature(&update(2, colors[1], 0))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 2);
    boxed(api, info, FAMILY, dev, Blink)
}

pub static DETECTORS: &[Detector] = &[det(
    "ThingM blink(1) mk2",
    0x27B8,
    0x01ED,
    ANY,
    0xFF00,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // BlinkController::SendUpdate for LED A (value 1) and LED B (value 2)
    #[test]
    fn two_leds() {
        let a = [Color(1, 2, 3), Color(4, 5, 6)];
        let [first, _, next] = pushes(Blink, 2, vec![], &a, &[Color(7, 7, 7), Color(4, 5, 6)]);
        assert_eq!(
            first,
            vec![
                Op::F(vec![1, 0x63, 1, 2, 3, 0, 0, 1, 0]),
                Op::F(vec![1, 0x63, 4, 5, 6, 0, 0, 2, 0])
            ]
        );
        assert_eq!(next[0], Op::F(vec![1, 0x63, 7, 7, 7, 0, 0, 1, 0]));
        assert_eq!(update(1, Color(0, 0, 0), 0x1234)[5..7], [0x12, 0x34]);
    }
}
