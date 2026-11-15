//! Ported from OpenRGB Controllers/ColorfulTuringGPUController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::COLORFUL_TURING;
use super::common::{Driver, Gpu, Io, detect_each, device_id, row};

const COLOR_PACKET_LENGTH: usize = 8;

// DetectColorfulTuringGPUControllers has no probe. The constructor's raw I2C reads only fill
// the UI, so they are left out.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, COLORFUL_TURING, |bus, card| {
        let dev = row(device_id("colorful-turing", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(bus, dev, Turing { addr: card.addr }))
    })
}

/// SetDirect(color, false) packet with its checksum.
fn direct_packet(c: Color) -> [u8; COLOR_PACKET_LENGTH] {
    let mut p = [0x08, 0x00, 0x20, 0x10, c.0, c.1, c.2, 0];
    let sum = p[..COLOR_PACKET_LENGTH - 1]
        .iter()
        .fold(1u32, |a, &b| a + b as u32);
    p[COLOR_PACKET_LENGTH - 1] = (256 - (sum & 0xFF)) as u8;
    p
}

struct Turing {
    addr: u8,
}

impl Driver for Turing {
    // DeviceUpdateMode only calls DeviceUpdateLEDs.
    fn setup(&mut self, _: &Io, _: &[Color]) {}

    // A raw I2C write of the packet is the same on the wire as a block write whose register
    // byte is the packet's first byte.
    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        let p = direct_packet(colors[0]);
        io.write_block(self.addr, p[0], &p[1..]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    // ColorfulTuringGPUController::SetDirect checksum: 256 - ((1 + sum) & 0xFF).
    #[test]
    fn checksum() {
        // 1 + 0x08 + 0x20 + 0x10 + 0x10 + 0x20 + 0x30 = 0x99.
        assert_eq!(direct_packet(Color(0x10, 0x20, 0x30))[7], 0x67);
        // 0x39 + 3 * 0xFF = 0x336 -> 0x36 -> 0xCA.
        assert_eq!(direct_packet(Color(0xFF, 0xFF, 0xFF))[7], 0xCA);
        // A sum ending in 0x00 gives 256, cut to 0.
        assert_eq!(direct_packet(Color(0xC7, 0, 0))[7], 0x00);
    }

    // RGBController_ColorfulTuringGPU::DeviceUpdateLEDs in Direct.
    #[test]
    fn direct_and_frame() {
        let card = "iGame GeForce RTX 2070 SUPER Advanced OC-V";
        let bus = Fake::new("NV", ids(COLORFUL_TURING, card)).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert!(take().is_empty());
        assert_eq!(
            push(&mut b, &[Color(0x10, 0x20, 0x30)]),
            s(&["wblk 50 08 00 20 10 10 20 30 67"])
        );
    }
}
