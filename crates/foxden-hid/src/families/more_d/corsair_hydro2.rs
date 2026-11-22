// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::winusb::WinUsb;
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "corsair-hydro2";
const VID: u16 = 0x1B1C;
const PID: u16 = 0x0C09;
const NAME: &str = "Corsair H100i v2";

/// SetLED's packet; an all-black colour turns the LED off, as black alone leaves it glowing.
pub fn led_packet(c: Color) -> [u8; 19] {
    let on = (c.0 as u16 + c.1 as u16 + c.2 as u16 != 0) as u8;
    [
        0x10, c.0, c.1, c.2, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x0A, 0x05, on, 0, 0, 0, 0,
        0x01,
    ]
}

fn init(io: &mut dyn Io) -> Result<(), String> {
    io.reset()?;
    io.control(0x40, 0, 0xFFFF, 0x0000)?;
    io.control(0x40, 2, 0x0002, 0x0000)?;
    io.control(0x40, 1, 0x0002, 0x0000)?;
    io.control(0x40, 4, 0x0002, 0x0000)
}

pub struct Hydro2;

impl Proto for Hydro2 {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        init(io)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&led_packet(colors[0]))?;
        let got = io.read(&mut [0; 32], 1000).unwrap_or(0);
        if got != 32 {
            // Reinitialises after sleep
            init(io)?;
        }
        Ok(())
    }
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    let Ok(io) = WinUsb::open(VID, PID, 0x02, 0x82) else {
        return Vec::new();
    };
    let id = format!("hid:{FAMILY}-{VID:04x}{PID:04x}");
    let dev = rows(id, NAME, DeviceKind::Other, &[1]);
    vec![boxed(Box::new(io), dev, FAMILY, Hydro2)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn inits() -> Vec<Op> {
        vec![
            Op::Reset,
            Op::Ctl(0x40, 0, 0xFFFF, 0),
            Op::Ctl(0x40, 2, 2, 0),
            Op::Ctl(0x40, 1, 2, 0),
            Op::Ctl(0x40, 4, 2, 0),
        ]
    }

    // SendInit from the constructor, SetLED, and SendInit again on a short reply
    #[test]
    fn init_led_and_reinit() {
        let [first, _, next] = pushes(
            Hydro2,
            1,
            vec![vec![0; 32]],
            &[Color(1, 2, 3)],
            &[Color(0, 0, 0)],
        );
        let mut want = inits();
        want.push(w(vec![
            0x10, 1, 2, 3, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0x0A, 0x05, 1, 0, 0, 0, 0, 1,
        ]));
        want.push(Op::R(32));
        assert_eq!(first, want);
        let mut want = vec![w(vec![
            0x10, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0x0A, 0x05, 0, 0, 0, 0, 0, 1,
        ])];
        want.push(Op::R(32));
        want.extend(inits());
        assert_eq!(next, want);
    }
}
