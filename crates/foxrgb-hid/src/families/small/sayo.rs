//! Ported from OpenRGB Controllers/SayoDeviceController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "sayo";
const LIGHTING_SET: u8 = 0x11;

pub fn packet(command: &[u8]) -> [u8; 64] {
    let mut sum: u16 = 0x1221;
    for pair in command.chunks(2) {
        let word = pair[0] as u16 | (*pair.get(1).unwrap_or(&0) as u16) << 8;
        sum = sum.wrapping_add(word);
    }
    let mut p = [0u8; 64];
    p[..4].copy_from_slice(&[0x21, 0x12, sum as u8, (sum >> 8) as u8]);
    p[4..4 + command.len()].copy_from_slice(command);
    p
}

pub fn set_mode(mode: u8, speed: u8, c: Color, random: bool) -> [u8; 64] {
    let mode_byte = (speed & 3) << 6 | (random as u8) << 4 | (mode & 0xF);
    packet(&[
        0x1C,
        0x00,
        LIGHTING_SET,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x15,
        0x00,
        0x28,
        0x00,
        0x26,
        0x00,
        0x4C,
        0x00,
        0x26,
        0x00,
        0x00,
        0x00,
        mode_byte,
        0x00,
        0x80,
        0x80,
        c.0,
        c.1,
        c.2,
    ])
}

struct Sayo;

impl Proto for Sayo {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        // Direct goes out as static with speed 3 - 0
        io.write(&set_mode(0x00, 3, colors[0], false))?;
        let mut buf = [0u8; 64];
        while io.read(&mut buf, 0)? > 0 {}
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Keyboard, 1);
    boxed(api, info, FAMILY, dev, Sayo)
}

pub static DETECTORS: &[Detector] = &[det(
    "SayoDevice E1",
    0x8089,
    0x0007,
    ANY,
    0xFF11,
    0x0002,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SayoDeviceController::SetMode -> SendPacket with flush
    #[test]
    fn static_packet_and_flush() {
        let [first, _, _] = pushes(Sayo, 1, vec![vec![1; 64]], &[Color(1, 2, 3)], &[]);
        let words: u32 = 0x1221
            + 0x001C
            + 0x0011
            + 0x0001
            + 0x0015
            + 0x0028
            + 0x0026
            + 0x004C
            + 0x0026
            + 0x00C0
            + 0x8080
            + 0x0201
            + 0x0003;
        let mut e = pad(&[0x21, 0x12, words as u8, (words >> 8) as u8], 4);
        e.extend([
            0x1C, 0, 0x11, 0, 1, 0, 0, 0, 0x15, 0, 0x28, 0, 0x26, 0, 0x4C, 0, 0x26, 0, 0, 0, 0xC0,
            0, 0x80, 0x80, 1, 2, 3,
        ]);
        e.resize(64, 0);
        assert_eq!(first, vec![w(e), Op::Read, Op::Read]);
        assert_eq!(
            packet(&[0x06, 0x00, 0x0d, 0x00, 0x96, 0x72])[2..4],
            [0xCA, 0x84]
        );
    }
}
