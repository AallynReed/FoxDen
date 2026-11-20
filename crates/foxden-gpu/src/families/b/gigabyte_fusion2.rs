//! Ported from OpenRGB Controllers/GigabyteRGBFusion2GPUController (GPL-2.0-or-later).

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::FUSION2;
use super::common::{
    AMD_GPU_VEN, Driver, Gpu, Io, detect_each, device_id, info, is_amd_gpu_i2c_bus, row,
};

const REG_COLOR: u8 = 0x40;
const REG_MODE: u8 = 0x88;
const REG_COLOR_LEFT_MID: u8 = 0xB0;
const REG_COLOR_RIGHT: u8 = 0xB1;
const MODE_STATIC: u8 = 0x01;
const BRIGHTNESS_MAX: u8 = 0x63;
const ZONES: u32 = 5;

/// TestForGigabyteRGBFusion2GPUController
fn test_for(io: &Io, addr: u8) -> bool {
    io.raw_write(addr, &[0xAB, 0, 0, 0, 0, 0, 0, 0]);
    match io.raw_read(addr, 4) {
        Some(d) => d[0] == 0xAB && d[1..] != [0xAB; 3],
        None => false,
    }
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    let i = info(bus);
    if i.pci_vendor == AMD_GPU_VEN && !is_amd_gpu_i2c_bus(&i) {
        return Vec::new();
    }
    detect_each(bus, FUSION2, |bus, card| {
        if !test_for(&Io::new(bus), card.addr) {
            return None;
        }
        let dev = row(
            device_id("gigabyte-fusion2", bus, card.addr),
            card.name,
            ZONES,
        );
        let driver = Fusion2 {
            addr: card.addr,
            zone_color: [Color::BLACK; 4],
        };
        Some(Gpu::boxed(bus, dev, driver))
    })
}

struct Fusion2 {
    addr: u8,
    /// RGBFusion2GPUController::zone_color, never cleared.
    zone_color: [Color; 4],
}

impl Fusion2 {
    // RGBFusion2GPUController::SetMode for Direct; DeviceUpdateLEDs never sets the speed, sent as 0.
    fn zone(&mut self, io: &Io, zone: u8, c: Color) {
        if zone < 4 {
            self.zone_color[zone as usize] = c;
        }
        let mode = [REG_MODE, MODE_STATIC, 0, BRIGHTNESS_MAX, 0, zone + 1, 0, 0];
        io.raw_write(self.addr, &mode);
        let [a, b, z2, _] = self.zone_color;
        let colors = match zone {
            0 | 1 => [
                REG_COLOR_LEFT_MID,
                MODE_STATIC,
                a.0,
                a.1,
                a.2,
                b.0,
                b.1,
                b.2,
            ],
            2 => [REG_COLOR_RIGHT, MODE_STATIC, z2.0, z2.1, z2.2, 0, 0, 0],
            _ => [REG_COLOR, c.0, c.1, c.2, zone + 1, 0, 0, 0],
        };
        io.raw_write(self.addr, &colors);
    }
}

impl Driver for Fusion2 {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // RGBController_RGBFusion2GPU::DeviceUpdateLEDs: every zone, every time
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        for (zone, &c) in colors.iter().enumerate() {
            self.zone(io, zone as u8, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "Gigabyte AORUS GeForce RTX 2060 SUPER";

    fn bus(reply: &[u8]) -> SharedBus {
        Fake::new("NV", ids(FUSION2, CARD))
            .raw(0x50, &[reply])
            .shared()
    }

    // TestForGigabyteRGBFusion2GPUController: AB first, and not an all-AB echo.
    #[test]
    fn detect_reply() {
        assert_eq!(
            run(&[super::detect], &bus(&[0xAB, 0x11, 0x52, 0x03])).len(),
            1
        );
        assert_eq!(take(), s(&["wraw 50 ab 00 00 00 00 00 00 00", "rraw 50 4"]));
        assert!(run(&[super::detect], &bus(&[0xAB; 4])).is_empty());
        assert!(run(&[super::detect], &bus(&[0, 0, 0, 0])).is_empty());
    }

    // RGBFusion2GPUController::SetMode for zones 0 to 4 in static at brightness 0x63.
    #[test]
    fn direct_zones() {
        let mut b = run(&[super::detect], &bus(&[0xAB, 0x10, 0x01, 0x02])).remove(0);
        let c = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
            Color(13, 14, 15),
        ];
        let w = |body: &str| format!("wraw 50 {body}");
        assert_eq!(
            push(&mut b, &c),
            vec![
                w("88 01 00 63 00 01 00 00"),
                w("b0 01 01 02 03 00 00 00"),
                w("88 01 00 63 00 02 00 00"),
                w("b0 01 01 02 03 04 05 06"),
                w("88 01 00 63 00 03 00 00"),
                w("b1 01 07 08 09 00 00 00"),
                w("88 01 00 63 00 04 00 00"),
                w("40 0a 0b 0c 04 00 00 00"),
                w("88 01 00 63 00 05 00 00"),
                w("40 0d 0e 0f 05 00 00 00"),
            ]
        );
        let log = push(&mut b, &[Color(9, 9, 9), c[1], c[2], c[3], c[4]]);
        assert_eq!(log[1], w("b0 01 09 09 09 04 05 06"));
    }
}
