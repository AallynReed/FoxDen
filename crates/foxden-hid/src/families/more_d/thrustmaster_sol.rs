// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::winusb::WinUsb;
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "thrustmaster-sol";
const VID: u16 = 0x044F;
const INTERFACE: u8 = 1;
const EP_OUT: u8 = 0x02;
const EP_IN: u8 = 0x82;
const PACKET: usize = 64;
const MAX_ENTRIES: usize = 15;
const GRIP: u16 = 0x100;
const MARKER: u8 = 0xFF;

pub const DEVICES: &[(u16, &str)] = &[
    (0x0422, "Thrustmaster Sol-R Right"),
    (0x042A, "Thrustmaster Sol-R Left"),
    (0x0420, "Thrustmaster Sol F16 Right"),
    (0x0421, "Thrustmaster Sol F18 Right"),
    (0x0428, "Thrustmaster Sol F16 Left"),
    (0x0429, "Thrustmaster Sol F18 Left"),
];

/// Zone value of each LED: thumbstick, TM logo, left buttons, base ring, right buttons.
#[rustfmt::skip]
pub const ZONES: [u16; 21] = [
    GRIP,
    0x01, 0x02, 0x03, 0x00,
    0x11, 0x10, 0x12, 0x13,
    0x04, 0x05, 0x06, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
    0x07, 0x08, 0x0A, 0x09,
];

const W: usize = 16;

/// The zone matrices side by side, a blank column between each.
#[rustfmt::skip]
pub fn layout() -> Vec<u32> {
    const N: u32 = NA;
    vec![
        0, N,  1,  4, N,  5,  6, N, N, 16,  9, 10, N, N, 17, 18,
        N, N,  2,  3, N,  7,  8, N, 15, N,  N,  N, 11, N, 19, 20,
        N, N,  N,  N, N,  N,  N, N, N, 14, 13, 12, N, N,  N,  N,
    ]
}

/// BuildAndSendPackets, volatile: a grip packet per grip LED, base LEDs 15 to a packet.
pub fn packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut base = Vec::new();
    for (&zone, c) in ZONES.iter().zip(colors) {
        if zone & GRIP != 0 {
            let mut p = vec![0u8; PACKET];
            p[..8].copy_from_slice(&[0x01, 0x88, 0x01, MARKER, zone as u8, c.0, c.1, c.2]);
            out.push(p);
        } else {
            base.push((zone as u8, c));
        }
    }
    for chunk in base.chunks(MAX_ENTRIES) {
        let mut p = vec![0u8; PACKET];
        p[..4].copy_from_slice(&[0x01, 0x08, chunk.len() as u8, MARKER]);
        for (j, (zone, c)) in chunk.iter().enumerate() {
            p[4 + j * 4..8 + j * 4].copy_from_slice(&[*zone, c.0, c.1, c.2]);
        }
        out.push(p);
    }
    out
}

pub struct Sol;

impl Proto for Sol {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in packets(colors) {
            io.write(&p)?;
            io.sleep(2);
        }
        Ok(())
    }
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    DEVICES
        .iter()
        .filter_map(|&(pid, name)| {
            let io = WinUsb::open_interface(VID, pid, INTERFACE, EP_OUT, EP_IN).ok()?;
            let id = format!("hid:{FAMILY}-{VID:04x}{pid:04x}");
            let dev = mapped(id, name, DeviceKind::Other, &layout(), W, ZONES.len());
            Some(boxed(Box::new(io), dev, FAMILY, Sol))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout_places_every_led() {
        let map = layout();
        assert_eq!(map.len(), 3 * W);
        let mut used: Vec<u32> = map.iter().copied().filter(|&m| m != NA).collect();
        used.sort();
        assert_eq!(used, (0..21).collect::<Vec<_>>());
    }

    #[test]
    fn direct_frame() {
        let a: Vec<Color> = (0..21u8).map(|i| Color(i, i + 100, i + 200)).collect();
        let [first, again, _] = pushes(Sol, 21, vec![], &a, &[]);
        let mut base1 = vec![0x01, 0x08, 15, 0xFF];
        for (i, z) in [
            1u8, 2, 3, 0, 0x11, 0x10, 0x12, 0x13, 4, 5, 6, 0x0B, 0x0C, 0x0D, 0x0E,
        ]
        .into_iter()
        .enumerate()
        {
            let i = i as u8 + 1;
            base1.extend([z, i, i + 100, i + 200]);
        }
        let mut base2 = vec![0x01, 0x08, 5, 0xFF];
        for (i, z) in [0x0Fu8, 7, 8, 0x0A, 9].into_iter().enumerate() {
            let i = i as u8 + 16;
            base2.extend([z, i, i + 100, i + 200]);
        }
        assert_eq!(
            first,
            vec![
                w(pad(&[0x01, 0x88, 0x01, 0xFF, 0x00, 0, 100, 200], 64)),
                Op::Sleep(2),
                w(base1),
                Op::Sleep(2),
                w(pad(&base2, 64)),
                Op::Sleep(2),
            ]
        );
        assert!(again.is_empty());
    }
}
