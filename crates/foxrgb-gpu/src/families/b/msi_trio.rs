//! MSI RTX 50 GAMING TRIO cards, per LED (protocol learned from SignalRGB's MSI_GPU_Blackwell.js).

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_smbus::i2c::SharedBus;

use super::common::{Card, Driver, Gpu, Io, RECT, c, detect_each, device_id};

const REG_FLAGS_50: u8 = 0x46;
const REG_STRIP: u8 = 0x06;
const REG_EFFECT: u8 = 0x20;
const REG_LEDS: u8 = 0x04;
const INIT: [(u8, u8); 3] = [(REG_FLAGS_50, 1), (REG_STRIP, 0), (REG_EFFECT, 0)];
/// Three logo LEDs, then the fan strip, on a 10 x 2 grid.
const GRID: [(u8, u8); 6] = [(0, 0), (1, 0), (2, 0), (8, 0), (9, 0), (7, 1)];

#[rustfmt::skip]
pub const MSI_BLACKWELL_TRIO: &[Card] = &[
    c("MSI GeForce RTX 5060 GAMING TRIO", 0x10DE, 0x2D05, 0x1462, 0x5372, 0x68),
    c("MSI GeForce RTX 5060 Ti GAMING TRIO", 0x10DE, 0x2D04, 0x1462, 0x5353, 0x68),
    c("MSI GeForce RTX 5060 Ti GAMING TRIO OC", 0x10DE, 0x2D04, 0x1462, 0x5360, 0x68),
    c("MSI GeForce RTX 5060 Ti GAMING OC", 0x10DE, 0x2D04, 0x1462, 0x5361, 0x68),
    c("MSI GeForce RTX 5060 Ti GAMING OC", 0x10DE, 0x2D04, 0x1462, 0x5354, 0x68),
    c("MSI GeForce RTX 5070 GAMING TRIO", 0x10DE, 0x2F04, 0x1462, 0x5322, 0x68),
    c("MSI GeForce RTX 5070 Ti GAMING TRIO", 0x10DE, 0x2C05, 0x1462, 0x5315, 0x68),
    c("MSI GeForce RTX 5070 Ti MLG Edition", 0x10DE, 0x2C05, 0x1462, 0x5318, 0x68),
    c("MSI GeForce RTX 5080 GAMING TRIO", 0x10DE, 0x2C02, 0x1462, 0x5315, 0x68),
    c("MSI GeForce RTX 5090 GAMING TRIO", 0x10DE, 0x2B85, 0x1462, 0x5303, 0x68),
];

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, MSI_BLACKWELL_TRIO, |bus, card| {
        let device = Device {
            id: device_id("msi-trio", bus, card.addr),
            name: card.name.into(),
            kind: DeviceKind::Gpu,
            rect: RECT,
            grid: [10, 2],
            leds: GRID
                .iter()
                .map(|&(x, y)| [(x as f32 + 0.5) / 10.0, (y as f32 + 0.5) / 2.0])
                .collect(),
        };
        Some(Gpu::boxed(bus, device, Trio { addr: card.addr }))
    })
}

/// 0x01, then index, red, green, blue, 0x01 for each LED.
fn block(colors: &[Color]) -> Vec<u8> {
    let mut data = vec![0x01];
    for (i, c) in colors.iter().enumerate() {
        data.extend([i as u8, c.0, c.1, c.2, 0x01]);
    }
    data
}

struct Trio {
    addr: u8,
}

impl Driver for Trio {
    fn setup(&mut self, io: &Io, _: &[Color]) {
        for (reg, v) in INIT {
            io.write(self.addr, reg, v);
        }
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        io.write_block(self.addr, REG_LEDS, &block(colors));
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s};
    use super::*;

    #[test]
    fn per_led_frame() {
        let card = "MSI GeForce RTX 5080 GAMING TRIO";
        let bus = Fake::new("NV", ids(MSI_BLACKWELL_TRIO, card)).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(b.devices()[0].leds.len(), 6);
        let mut colors = [Color::BLACK; 6];
        colors[0] = Color(1, 2, 3);
        colors[5] = Color(4, 5, 6);
        let frame = "wblk 68 04 01 00 01 02 03 01 01 00 00 00 01 02 00 00 00 01 \
                     03 00 00 00 01 04 00 00 00 01 05 04 05 06 01";
        assert_eq!(
            push(&mut b, &colors),
            s(&["wr 68 46 01", "wr 68 06 00", "wr 68 20 00", frame])
        );
        assert_eq!(block(&colors).len(), 31);
    }

    // The 5070 Ti and the 5080 share subsystem 0x5315 on different chips.
    #[test]
    fn shared_subsystem_picks_by_chip() {
        let bus = Fake::new("NV", [0x10DE, 0x2C05, 0x1462, 0x5315]).shared();
        let found = run(&[detect], &bus);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].devices()[0].name,
            "MSI GeForce RTX 5070 Ti GAMING TRIO"
        );
    }
}
