//! Ported from OpenRGB Controllers/HYTEMousematController (GPL-2.0-or-later).

use super::common::*;
use super::serial::{Port, Serial};
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "hyte-cnvs";
pub const IDS: [(u16, u16); 2] = [(0x3402, 0x0B00), (0x3402, 0x0B01)];
const BAUD: u32 = 2_000_000;
const LEDS: usize = 50;
const MAX_BRIGHTNESS: u32 = 72;

/// StreamingCommand: 50 LEDs in GRB order, scaled to 72%.
pub fn stream_packet(colors: &[Color]) -> [u8; 157] {
    let mut b = [0u8; 157];
    b[..7].copy_from_slice(&[0xFF, 0xEE, 0x02, 0x01, 0x00, 0x32, 0x00]);
    let scale = |v: u8| (MAX_BRIGHTNESS * v as u32 / 100) as u8;
    for (i, c) in colors.iter().take(LEDS).enumerate() {
        b[7 + i * 3..10 + i * 3].copy_from_slice(&[scale(c.1), scale(c.0), scale(c.2)]);
    }
    b
}

pub struct Cnvs;

impl Proto for Cnvs {
    // Direct mode: firmware animation off
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[0xFF, 0xDC, 0x05, 0x00]).map(|_| ())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&stream_packet(colors)).map(|_| ())
    }
}

pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let io = Serial::open(&port.name, BAUD, false).ok()?;
    let dev = rows(
        named_id(FAMILY, &port.name),
        "HYTE CNVS",
        DeviceKind::Other,
        &[LEDS],
    );
    Some(boxed(Box::new(io), dev, FAMILY, Cnvs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn animation_off_then_stream() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(100, 200, 255);
        a[49] = Color(1, 2, 3);
        let [first, _, _] = pushes(Cnvs, LEDS, vec![], &a, &[]);
        assert_eq!(first[0], w(vec![0xFF, 0xDC, 0x05, 0x00]));
        let mut s = pad(&[0xFF, 0xEE, 2, 1, 0, 0x32, 0, 144, 72, 183], 157);
        s[154..157].copy_from_slice(&[1, 0, 2]);
        assert_eq!(first[1], w(s));
    }
}
