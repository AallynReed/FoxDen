//! Ported from OpenRGB Controllers/ColorfulGPUController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::COLORFUL;
use super::common::{Driver, Gpu, Io, detect_each, device_id, info, row};

const PACKET_V2: usize = 122;
const VULCAN_OCV: u16 = 0x1200;
const NEPTUNE_OCV: u16 = 0x1301;

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, COLORFUL, |bus, card| {
        let io = Io::new(bus);
        io.raw_write(card.addr, &[0xAA, 0xEF, 0x81, 0x02, 0x1C, 0x02]);
        let reply = io.raw_read(card.addr, 0x40)?;
        if reply[..3] != [0xAA, 0xEF, 0x81] {
            return None;
        }
        let v2 = matches!(info(bus).pci_subsystem_device, VULCAN_OCV | NEPTUNE_OCV);
        let dev = row(device_id("colorful", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(
            bus,
            dev,
            Colorful {
                addr: card.addr,
                v2,
            },
        ))
    })
}

struct Colorful {
    addr: u8,
    v2: bool,
}

/// The packet with its sum in the last two bytes, low byte first.
fn summed(mut pkt: Vec<u8>) -> Vec<u8> {
    let n = pkt.len();
    let crc: u32 = pkt[..n - 2].iter().map(|&b| b as u32).sum();
    pkt[n - 2] = crc as u8;
    pkt[n - 1] = (crc >> 8) as u8;
    pkt
}

impl Driver for Colorful {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // ColorfulGPUController::SetDirect
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let Color(r, g, b) = colors[0];
        let pkt = if self.v2 {
            let mut pkt = vec![0; PACKET_V2];
            pkt[..6].copy_from_slice(&[0xAA, 0xEF, 0x01, 0x04, 0x88, 0x26]);
            for i in (6..PACKET_V2 - 2).step_by(3) {
                pkt[i..i + 3].copy_from_slice(&[r, g, b]);
            }
            pkt
        } else {
            vec![0xAA, 0xEF, 0x12, 0x03, 0x01, 0xFF, r, g, b, 0, 0]
        };
        io.raw_write(self.addr, &summed(pkt));
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "iGame GeForce RTX 3070 Advanced OC-V";
    const HELLO: &[u8] = &[0xAA, 0xEF, 0x81, 0x03];

    // TestForColorfulGPU: the handshake write, then a 64-byte read that must echo AA EF 81.
    #[test]
    fn detect_handshake() {
        let bus = Fake::new("NV", ids(COLORFUL, CARD))
            .raw(0x61, &[HELLO])
            .shared();
        assert_eq!(run(&[super::detect], &bus).len(), 1);
        assert_eq!(take(), s(&["wraw 61 aa ef 81 02 1c 02", "rraw 61 64"]));
        let wrong = Fake::new("NV", ids(COLORFUL, CARD))
            .raw(0x61, &[&[0xAA, 0xEF, 0x80]])
            .shared();
        assert!(run(&[super::detect], &wrong).is_empty());
    }

    // ColorfulGPUController::SetDirect, the 11-byte packet: 0x030E is the sum of the first nine.
    #[test]
    fn direct_v1() {
        let bus = Fake::new("NV", ids(COLORFUL, CARD))
            .raw(0x61, &[HELLO])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        assert_eq!(
            push(&mut b, &[Color(0x10, 0x20, 0x30)]),
            s(&["wraw 61 aa ef 12 03 01 ff 10 20 30 0e 03"])
        );
    }

    // ColorfulGPUController::SetDirect, the 122-byte packet for the 4070 Vulcan and 4090 Neptune.
    #[test]
    fn direct_v2() {
        let card = "iGame GeForce RTX 4090 Neptune OC-V";
        let bus = Fake::new("NV", ids(COLORFUL, card))
            .raw(0x61, &[HELLO])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        let log = push(&mut b, &[Color(1, 2, 3)]);
        let mut expect = String::from("wraw 61 aa ef 01 04 88 26");
        for _ in 0..38 {
            expect.push_str(" 01 02 03");
        }
        // 0x24C from the header plus 38 * 6
        expect.push_str(" 30 03");
        assert_eq!(log, vec![expect]);
    }
}
