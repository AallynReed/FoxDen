//! Ported from OpenRGB Controllers/BlinkyTapeController (GPL-2.0-or-later).

use super::common::*;
use super::serial::{Port, Serial};
use foxrgb_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "blinkytape";
pub const IDS: (u16, u16) = (0x1D50, 0x605E);
const BAUD: u32 = 115_200;
/// OpenRGB leaves the length to the user; a stock BlinkyTape has 60 LEDs.
const LEDS: usize = 60;

/// SetLEDs: RGB capped at 254, then the 0xFF that ends a frame.
pub fn frame_packet(colors: &[Color]) -> Vec<u8> {
    let mut b: Vec<u8> = colors
        .iter()
        .flat_map(|c| [c.0.min(254), c.1.min(254), c.2.min(254)])
        .collect();
    b.push(0xFF);
    b
}

pub struct BlinkyTape;

impl Proto for BlinkyTape {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&frame_packet(colors)).map(|_| ())
    }
}

pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let io = Serial::open(&port.name, BAUD, false).ok()?;
    let dev = rows(
        named_id(FAMILY, &port.name),
        "BlinkyTape",
        DeviceKind::Strip,
        &[LEDS],
    );
    Some(boxed(Box::new(io), dev, FAMILY, BlinkyTape))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn frame_ends_with_ff() {
        let a = [Color(255, 1, 2), Color(3, 4, 255)];
        let [first, _, _] = pushes(BlinkyTape, 2, vec![], &a, &[]);
        assert_eq!(first, vec![w(vec![254, 1, 2, 3, 4, 254, 0xFF])]);
    }
}
