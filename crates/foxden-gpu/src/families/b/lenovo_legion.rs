//! Lenovo Legion graphics cards.

use std::time::Duration;

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::common::{Card, Driver, Gpu, Io, c, detect_each, device_id, row, sleep};

/// Supported Lenovo Legion cards.
#[rustfmt::skip]
pub const LENOVO_LEGION: &[Card] = &[
    c("Lenovo Legion GeForce RTX 5070 Ti", 0x10DE, 0x2C05, 0x17AA, 0xC773, 0x49),
    c("Lenovo Legion GeForce RTX 5080", 0x10DE, 0x2C02, 0x17AA, 0xC770, 0x49),
];

const REG_FRAME: u8 = 0x14;
const REG_EFFECT: u8 = 0x15;
const REG_UNKNOWN: u8 = 0x16;
const REG_BRIGHTNESS: u8 = 0x17;
const REG_RED: u8 = 0x18;
const REG_BEGIN: u8 = 0x30;
const REG_COMMIT: u8 = 0x31;
const REG_APPLY: u8 = 0x32;
const REG_SELECT: u8 = 0x40;
const REG_LOGO: u8 = 0x50;
const BEGIN: u8 = 0xB0;
const COMMIT: u8 = 0xB1;
const APPLY: u8 = 0xB2;
const EFFECT_STATIC: u8 = 0x01;
/// Brightness is a percentage.
const FULL: u8 = 100;
/// Registers that hold these values in every captured update.
const CONSTANTS: [(u8, u8); 7] = [
    (0x1B, 0x00),
    (0x1C, 0xC8),
    (0x1D, 0xFF),
    (0x20, 0x00),
    (0x21, 0x00),
    (0x22, 0x00),
    (0x23, 0x00),
];
/// The controller drops writes closer together than this.
const SPACING: Duration = Duration::from_millis(15);
/// The plugin's default for its NVIDIA logo option.
const LOGO_ON: bool = true;

// No probe: the card is matched on its PCI ids alone. One colour for strip and backplate.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, LENOVO_LEGION, |bus, card| {
        let dev = row(device_id("lenovo-legion", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(bus, dev, Legion { addr: card.addr }))
    })
}

/// BuildColorSequence plus four logo frames: 37 register writes per colour change.
fn sequence(c: Color, logo: bool) -> Vec<(u8, u8)> {
    let mut w = vec![
        (REG_BEGIN, BEGIN),
        (REG_SELECT, 0x01),
        (REG_EFFECT, EFFECT_STATIC),
        (REG_UNKNOWN, 0x00),
        (REG_BRIGHTNESS, FULL),
        (REG_RED, c.0),
        (REG_RED + 1, c.1),
        (REG_RED + 2, c.2),
    ];
    w.extend(CONSTANTS);
    w.extend([
        (REG_COMMIT, COMMIT),
        (REG_APPLY, APPLY),
        (REG_BEGIN, BEGIN),
        (REG_SELECT, 0x01),
        (REG_FRAME, 0x01),
        (REG_COMMIT, COMMIT),
    ]);
    for _ in 0..4 {
        w.extend([
            (REG_BEGIN, BEGIN),
            (REG_LOGO, logo as u8),
            (REG_COMMIT, COMMIT),
            (REG_APPLY, APPLY),
        ]);
    }
    w
}

struct Legion {
    addr: u8,
}

impl Driver for Legion {
    fn setup(&mut self, _: &Io, _: &[Color]) {}

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        for (i, (reg, v)) in sequence(colors[0], LOGO_ON).into_iter().enumerate() {
            if i > 0 {
                sleep(SPACING);
            }
            io.write(self.addr, reg, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, take};
    use super::*;

    // Colour and logo sequences.
    #[test]
    fn colour_update_sequence() {
        let card = "Lenovo Legion GeForce RTX 5080";
        let bus = Fake::new("NV", ids(LENOVO_LEGION, card)).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert!(take().is_empty());
        let log = push(&mut b, &[Color(1, 2, 3)]);
        let writes: Vec<&String> = log.iter().filter(|l| l.starts_with("wr")).collect();
        assert_eq!(writes.len(), 37);
        assert_eq!(log.iter().filter(|l| *l == "sleep 15ms").count(), 36);
        assert_eq!(writes[0], "wr 49 30 b0");
        assert_eq!(writes[5..8], ["wr 49 18 01", "wr 49 19 02", "wr 49 1a 03"]);
        assert_eq!(writes[15..17], ["wr 49 31 b1", "wr 49 32 b2"]);
        assert_eq!(writes[19], "wr 49 14 01");
        assert_eq!(writes[22], "wr 49 50 01");
        assert_eq!(writes[36], "wr 49 32 b2");
        assert_eq!(sequence(Color::BLACK, false)[22], (REG_LOGO, 0));
    }
}
