// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::serial::{Port, Serial};
use foxden_core::{Backend, Color, Device, DeviceKind};

const FAMILY: &str = "blinkytape";
pub const IDS: (u16, u16) = (0x1D50, 0x605E);
const BAUD: u32 = 115_200;
/// The length is not reported; a stock BlinkyTape has 60 LEDs.
const LEDS: usize = 60;
const MAX_LEDS: usize = 512;

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

fn device(port: &str) -> Device {
    let id = named_id(FAMILY, port);
    let n = crate::lengths::length(&id, 0, "Strip", LEDS, 1, MAX_LEDS);
    rows(id, "BlinkyTape", DeviceKind::Strip, &[n])
}

pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let io = Serial::open(&port.name, BAUD, false).ok()?;
    Some(boxed(Box::new(io), device(&port.name), FAMILY, BlinkyTape))
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

    #[test]
    fn a_set_length_changes_the_led_count() {
        assert_eq!(device("COM91").leds.len(), 60);
        crate::lengths::set_one("hid:blinkytape-com92", 0, 144);
        assert_eq!(device("COM92").leds.len(), 144);
    }
}
