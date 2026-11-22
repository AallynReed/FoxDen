// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::serial::{Port, Serial};
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "arctic";
pub const IDS: (u16, u16) = (0x1A86, 0x7523);
const BAUD: u32 = 250_000;
const CHANNELS: usize = 4;
const HEADER: [u8; 8] = [0x01, 0x02, 0x03, 0xFF, 0x05, 0xFF, 0x02, 0x03];
const SET_RGB: u8 = 0x00;
const IDENTIFY: u8 = 0x5C;

/// SetChannels: header, command, then each channel capped at 254.
/// The whole packet is sent.
pub fn channels_packet(colors: &[Color]) -> Vec<u8> {
    let mut b = HEADER.to_vec();
    b.push(SET_RGB);
    for c in colors {
        b.extend([c.0.min(254), c.1.min(254), c.2.min(254)]);
    }
    b
}

/// IsPresent: the identify command answered with both checksums right.
pub fn is_present(io: &mut dyn Io) -> bool {
    let mut cmd = HEADER.to_vec();
    cmd.extend([IDENTIFY, 0x01, 0xFE, 0x01, 0xFE]);
    io.flush(true, false);
    if io.write(&cmd) != Ok(cmd.len()) {
        return false;
    }
    io.sleep(100);
    let mut r = [0u8; 15];
    if io.read(&mut r, 0) != Ok(r.len()) {
        return false;
    }
    let data = &r[1..13];
    let xor = data.iter().fold(0u8, |s, b| s ^ b);
    let add = data.iter().fold(0u8, |s, b| s.wrapping_add(*b));
    r[0] == IDENTIFY && r[13] == xor && r[14] == add
}

pub struct Arctic;

impl Proto for Arctic {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&channels_packet(colors)).map(|_| ())
    }
    // The controller needs to hear from us at least every second
    fn keepalive_ms(&self) -> Option<u64> {
        Some(500)
    }
}

pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let mut io = Serial::open(&port.name, BAUD, true).ok()?;
    io.set_dtr(true);
    if !is_present(&mut io) {
        return None;
    }
    let dev = rows(
        named_id(FAMILY, &port.name),
        "Arctic RGB Controller",
        DeviceKind::Strip,
        &[1; CHANNELS],
    );
    Some(boxed(Box::new(io), dev, FAMILY, Arctic))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn identify() {
        let mut reply = vec![IDENTIFY, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0xF0];
        let data = &reply[1..13];
        let xor = data.iter().fold(0u8, |s, b| s ^ b);
        let add = data.iter().fold(0u8, |s, b| s.wrapping_add(*b));
        reply.extend([xor, add]);
        let mut found = false;
        let ops = record(vec![reply.clone()], |io| found = is_present(io));
        assert!(found);
        assert_eq!(
            ops,
            vec![
                Op::Flush(true, false),
                w(vec![1, 2, 3, 0xFF, 5, 0xFF, 2, 3, 0x5C, 1, 0xFE, 1, 0xFE]),
                Op::Sleep(100),
                Op::R(15),
            ]
        );
        reply[14] ^= 1;
        record(vec![reply], |io| found = is_present(io));
        assert!(!found);
    }

    #[test]
    fn channels_capped_at_254() {
        let a = [
            Color(255, 1, 2),
            Color(3, 254, 255),
            Color::BLACK,
            Color(9, 9, 9),
        ];
        let [first, _, _] = pushes(Arctic, 4, vec![], &a, &[]);
        assert_eq!(
            first,
            vec![w(vec![
                1, 2, 3, 0xFF, 5, 0xFF, 2, 3, 0, 254, 1, 2, 3, 254, 254, 0, 0, 0, 9, 9, 9
            ])]
        );
    }
}
