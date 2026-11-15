//! Ported from OpenRGB Controllers/DygmaRaiseController (GPL-2.0-or-later).

use super::common::*;
use super::serial::{Port, Serial};
use foxrgb_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "dygma-raise";
pub const IDS: (u16, u16) = (0x1209, 0x2201);
const BAUD: u32 = 115_200;
const KEYS: u32 = 69;
const LEDS: usize = 69 + 62 + 1;
const W: usize = 14;

#[rustfmt::skip]
const KEYBOARD: [u32; 6 * W] = [
     0,  1,  2,  3,  4,  5,  6, 39, 38, 37, 36, 35, 34, 33,
     7,  8,  9, 10, 11, 12, 47, 46, 45, 44, 43, 42, 41, 40,
    13, 14, 15, 16, 17, 18, 54, 53, 52, 51, 50, 49, 48, NA,
    19, 20, 21, 22, 23, 24, 25, 60, 59, 58, 57, 56, 55, NA,
    26, 27, 28, NA, 29, 30, NA, 66, 65, NA, 64, 63, 62, 61,
    NA, NA, NA, NA, 31, 32, NA, 68, 67, NA, NA, NA, NA, NA,
];

#[rustfmt::skip]
const UNDERGLOW: [u32; 11 * W] = [
     2,  3,  4,  5,  6,  7, NA, 38, 37, 36, 35, 34, 33, 32,
     1, NA, NA, NA, NA,  8, NA, 39, NA, NA, NA, NA, NA, 31,
     0, NA, NA, NA, NA,  9, NA, 40, NA, NA, NA, NA, NA, 30,
    29, NA, NA, NA, NA, 10, NA, 41, NA, NA, NA, NA, NA, 61,
    28, NA, NA, NA, NA, 11, NA, 42, NA, NA, NA, NA, NA, 60,
    27, NA, NA, NA, NA, 12, NA, 43, NA, NA, NA, NA, NA, 59,
    26, NA, NA, NA, NA, 13, NA, 44, NA, NA, NA, NA, NA, 58,
    25, NA, NA, NA, NA, 14, NA, 45, NA, NA, NA, NA, NA, 57,
    24, NA, NA, NA, NA, 15, NA, 46, NA, NA, NA, NA, NA, 56,
    23, NA, NA, NA, NA, 16, NA, 47, NA, NA, NA, NA, NA, 55,
    22, 21, 20, 19, 18, 17, NA, 48, 49, 50, 51, 52, 53, 54,
];

/// Keys over underglow; the Neuron LED is left unmapped.
pub fn layout() -> Vec<u32> {
    let under = UNDERGLOW
        .iter()
        .map(|&m| if m == NA { NA } else { m + KEYS });
    KEYBOARD.iter().copied().chain(under).collect()
}

/// SendDirect: `led.theme` and each channel in decimal, then a newline.
pub fn theme_packet(colors: &[Color]) -> Vec<u8> {
    let mut s = String::from("led.theme");
    for c in colors {
        s += &format!(" {} {} {}", c.0, c.1, c.2);
    }
    s.push('\n');
    s.into_bytes()
}

pub struct Raise;

impl Proto for Raise {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let p = theme_packet(colors);
        let mut done = 0;
        while done < p.len() {
            match io.write(&p[done..])? {
                0 => return Err("serial write stalled".into()),
                n => done += n,
            }
        }
        Ok(())
    }
}

pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let io = Serial::open(&port.name, BAUD, false).ok()?;
    let id = named_id(FAMILY, &port.name);
    let dev = mapped(id, "Dygma Raise", DeviceKind::Keyboard, &layout(), W, LEDS);
    Some(boxed(Box::new(io), dev, FAMILY, Raise))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout_covers_keys_and_underglow() {
        let map = layout();
        assert_eq!(map.len(), 17 * W);
        let mut used: Vec<u32> = map.iter().copied().filter(|&m| m != NA).collect();
        used.sort();
        assert_eq!(used, (0..LEDS as u32 - 1).collect::<Vec<_>>());
        assert_eq!(map[6 * W], KEYS + 2);
        assert_eq!(map[16 * W + 13], KEYS + 54);
    }

    // snprintf(" %d") per channel after "led.theme", then "\n"
    #[test]
    fn theme() {
        let [first, again, next] = pushes(
            Raise,
            3,
            vec![],
            &[Color(0, 9, 10), Color(99, 100, 255), Color(7, 0, 42)],
            &[Color(1, 2, 3)],
        );
        assert_eq!(
            first,
            vec![w(b"led.theme 0 9 10 99 100 255 7 0 42\n".to_vec())]
        );
        assert!(again.is_empty());
        assert_eq!(next, vec![w(b"led.theme 1 2 3 0 0 0 0 0 0\n".to_vec())]);
    }
}
