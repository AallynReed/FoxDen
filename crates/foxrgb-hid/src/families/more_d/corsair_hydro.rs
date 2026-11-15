//! Ported from OpenRGB Controllers/CorsairHydroController (GPL-2.0-or-later).

use super::common::*;
use super::winusb::WinUsb;
use foxrgb_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "corsair-hydro";
const VID: u16 = 0x1B1C;

pub const DEVICES: &[(u16, &str)] = &[
    (0x0C15, "Corsair H100i PRO RGB"),
    (0x0C13, "Corsair H115i PRO RGB"),
    (0x0C12, "Corsair H150i PRO RGB"),
];

/// SendColors: two colours at least, the one pump colour twice.
pub fn colours_packet(colors: &[Color]) -> Vec<u8> {
    let mut out = vec![0x56, colors.len() as u8];
    for c in colors {
        out.extend([c.0, c.1, c.2]);
    }
    if colors.len() == 1 {
        out[1] = 2;
        out.extend([colors[0].0, colors[0].1, colors[0].2]);
    }
    out
}

fn bulk(io: &mut dyn Io, packet: &[u8], reply: usize) -> Result<(), String> {
    io.write(packet)?;
    io.read(&mut vec![0; reply], 1000)?;
    Ok(())
}

pub struct Hydro;

impl Proto for Hydro {
    // SendInit, then SendFirmwareRequest
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.control(0x40, 0x00, 0xFFFF, 0x0000)?;
        io.control(0x40, 0x02, 0x0002, 0x0000)?;
        bulk(io, &[0xAA], 7)
    }
    // SetFixed: the colours, then Apply Shift
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        bulk(io, &colours_packet(colors), 3)?;
        bulk(io, &[0x55, 0x01], 3)
    }
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    DEVICES
        .iter()
        .filter_map(|&(pid, name)| {
            let io = WinUsb::open(VID, pid, 0x01, 0x81).ok()?;
            let id = format!("hid:{FAMILY}-{VID:04x}{pid:04x}");
            let dev = rows(id, name, DeviceKind::Other, &[1]);
            Some(boxed(Box::new(io), dev, FAMILY, Hydro))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // Constructor's SendInit and SendFirmwareRequest, then SetFixed with one colour
    #[test]
    fn init_then_fixed() {
        let [first, _, next] = pushes(Hydro, 1, vec![], &[Color(1, 2, 3)], &[Color(4, 5, 6)]);
        assert_eq!(
            first,
            vec![
                Op::Ctl(0x40, 0x00, 0xFFFF, 0),
                Op::Ctl(0x40, 0x02, 0x0002, 0),
                w(vec![0xAA]),
                Op::R(7),
                w(vec![0x56, 2, 1, 2, 3, 1, 2, 3]),
                Op::R(3),
                w(vec![0x55, 0x01]),
                Op::R(3),
            ]
        );
        assert_eq!(next[0], w(vec![0x56, 2, 4, 5, 6, 4, 5, 6]));
    }

    #[test]
    fn two_colours_are_not_doubled() {
        assert_eq!(
            colours_packet(&[Color(1, 2, 3), Color(4, 5, 6)]),
            vec![0x56, 2, 1, 2, 3, 4, 5, 6]
        );
    }
}
