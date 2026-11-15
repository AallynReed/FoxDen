//! PNY XLR8 REVEL Lovelace graphics cards (protocol learned from SignalRGB's PNY_XLR8_REVEL_Lovelace_GPU.js).

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::common::{Card, Driver, Gpu, Io, c, detect_each, device_id, row};

/// PNY_XLR8_REVEL_Lovelace_GPU.js PNYGPUIDs.
#[rustfmt::skip]
pub const PNY_REVEL: &[Card] = &[
    c("PNY GeForce RTX 4090 XLR8 GAMING REVEL EPIC-X RGB", 0x10DE, 0x2684, 0x196E, 0x13C7, 0x49),
];

const REG_INIT: u8 = 0xEF;
const REG_CONTROL: u8 = 0xE0;
const REG_MODE: u8 = 0x60;
const REG_COLORS: u8 = 0xB0;
const LEDS: usize = 4;
/// Block colours mode: four header bytes, then RGB per LED.
const HEADER: [u8; 4] = [0x05; 4];
const LATCH: [u8; 3] = [0x05, 0x03, 0x18];

// No probe: the card is matched on its PCI ids alone.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, PNY_REVEL, |bus, card| {
        let dev = row(
            device_id("pny-revel", bus, card.addr),
            card.name,
            LEDS as u32,
        );
        Some(Gpu::boxed(bus, dev, Revel { addr: card.addr }))
    })
}

fn block(colors: &[Color]) -> Vec<u8> {
    let mut data = HEADER.to_vec();
    for c in colors.iter().take(LEDS) {
        data.extend([c.0, c.1, c.2]);
    }
    data
}

struct Revel {
    addr: u8,
}

impl Driver for Revel {
    fn setup(&mut self, io: &Io, _: &[Color]) {
        for reg in [REG_INIT, REG_CONTROL, REG_MODE] {
            io.write(self.addr, reg, 0x00);
        }
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        io.write_block(self.addr, REG_COLORS, &block(colors));
        io.write_block(self.addr, REG_CONTROL, &LATCH);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    // PNY_XLR8_REVEL_Lovelace_GPU.js Initialize and sendColors.
    #[test]
    fn setup_and_frame() {
        let card = "PNY GeForce RTX 4090 XLR8 GAMING REVEL EPIC-X RGB";
        let bus = Fake::new("NV", ids(PNY_REVEL, card)).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert!(take().is_empty());
        let colors = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        assert_eq!(
            push(&mut b, &colors),
            s(&[
                "wr 49 ef 00",
                "wr 49 e0 00",
                "wr 49 60 00",
                "wblk 49 b0 05 05 05 05 01 02 03 04 05 06 07 08 09 0a 0b 0c",
                "wblk 49 e0 05 03 18",
            ])
        );
        assert_eq!(push(&mut b, &[Color::BLACK; 4]).len(), 2);
    }
}
