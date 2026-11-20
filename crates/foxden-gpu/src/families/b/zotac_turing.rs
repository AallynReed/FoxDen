//! Ported from OpenRGB Controllers/ZotacTuringGPUController (GPL-2.0-or-later).

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::ZOTAC_TURING;
use super::common::{Driver, Gpu, Io, detect_each, device_id, row};

const REG_COLOR_AND_MODE: u8 = 0xA0;
const MODE_STATIC: u8 = 0x00;

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, ZOTAC_TURING, |bus, card| {
        let io = Io::new(bus);
        if !io.raw_write(card.addr, &[REG_COLOR_AND_MODE, 0xF1, 0x01, 0, 0, 0, 0, 0]) {
            return None;
        }
        // SetupInitialValues: Direct keeps the speed the card reports while it is in static.
        let speed = match io.raw_read(card.addr, 32) {
            Some(d) if d[0] == MODE_STATIC => d[5],
            _ => 0,
        };
        let dev = row(device_id("zotac-turing", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(
            bus,
            dev,
            Turing {
                addr: card.addr,
                speed,
            },
        ))
    })
}

struct Turing {
    addr: u8,
    speed: u8,
}

impl Driver for Turing {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // ZotacTuringGPUController::SetMode, then the read back that keeps the controller from hanging
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let Color(r, g, b) = colors[0];
        let pkt = [
            REG_COLOR_AND_MODE,
            0x00,
            MODE_STATIC,
            r,
            g,
            b,
            0x00,
            self.speed,
        ];
        io.raw_write(self.addr, &pkt);
        io.raw_read(self.addr, 32);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "ZOTAC GAMING GeForce RTX 2080 AMP";

    // TestForZotacTuringGPUController, then GetMode for the initial speed.
    #[test]
    fn detect_and_speed() {
        let bus = Fake::new("NV", ids(ZOTAC_TURING, CARD))
            .raw(0x49, &[&[0, 1, 2, 3, 0, 4]])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        assert_eq!(
            take(),
            s(&["wraw 49 a0 f1 01 00 00 00 00 00", "rraw 49 32"])
        );
        assert_eq!(
            push(&mut b, &[Color(9, 8, 7)]),
            s(&["wraw 49 a0 00 00 09 08 07 00 04", "rraw 49 32"])
        );
    }

    // A card in another mode leaves Direct at speed 0.
    #[test]
    fn other_mode_speed() {
        let bus = Fake::new("NV", ids(ZOTAC_TURING, CARD))
            .raw(0x49, &[&[3, 1, 2, 3, 0, 4]])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        assert_eq!(
            push(&mut b, &[Color(9, 8, 7)])[0],
            "wraw 49 a0 00 00 09 08 07 00 00"
        );
    }
}
