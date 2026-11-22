// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::PNY_LOVELACE;
use super::common::{Driver, Gpu, Io, detect_each, device_id, row};

const REG_LIGHTING: u8 = 0x02;
const MODE_STATIC: u8 = 0x06;

// No probe. LEDs: fan, right, left.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, PNY_LOVELACE, |bus, card| {
        let dev = row(device_id("pny-lovelace", bus, card.addr), card.name, 3);
        Some(Gpu::boxed(bus, dev, Lovelace { addr: card.addr }))
    })
}

struct Lovelace {
    addr: u8,
}

impl Driver for Lovelace {
    fn setup(&mut self, _: &Io, _: &[Color]) {}

    // Direct mode: a direct write for each of the three LEDs.
    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        for (i, c) in colors.iter().enumerate() {
            io.write_block(
                self.addr,
                REG_LIGHTING,
                &[MODE_STATIC, 0xFF, i as u8, 0x00, c.0, c.1, c.2],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    // Mode change with Direct active.
    #[test]
    fn direct_and_frame() {
        let card = "PNY GeForce RTX 4090 XLR8 VERTO";
        let bus = Fake::new("NV", ids(PNY_LOVELACE, card)).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert!(take().is_empty());
        let colors = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        assert_eq!(
            push(&mut b, &colors),
            s(&[
                "wblk 60 02 06 ff 00 00 01 02 03",
                "wblk 60 02 06 ff 01 00 04 05 06",
                "wblk 60 02 06 ff 02 00 07 08 09",
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(0, 0, 1), colors[1], colors[2]]).len(),
            3
        );
    }
}
