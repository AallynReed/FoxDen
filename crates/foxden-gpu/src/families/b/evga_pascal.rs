// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::EVGA_PASCAL;
use super::common::{Driver, Gpu, Io, detect_each, device_id, row};

const REG_MODE: u8 = 0x0C;
const REG_RED: u8 = 0x09;
const REG_GREEN: u8 = 0x0A;
const REG_BLUE: u8 = 0x0B;
const MODE_CUSTOM: u8 = 0x01;

// DetectEVGAPascalGPUControllers checks only port_id, which is always 1 on Windows buses.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, EVGA_PASCAL, |bus, card| {
        let dev = row(device_id("evga-v1", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(bus, dev, V1 { addr: card.addr }))
    })
}

struct V1 {
    addr: u8,
}

impl Driver for V1 {
    fn setup(&mut self, io: &Io, _: &[Color]) {
        io.write(self.addr, REG_MODE, MODE_CUSTOM);
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        let c = colors[0];
        io.write(self.addr, REG_RED, c.0);
        io.write(self.addr, REG_GREEN, c.1);
        io.write(self.addr, REG_BLUE, c.2);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    // DetectEVGAPascalGPUControllers: no probe, a device per matching card.
    #[test]
    fn detect_without_probe() {
        let bus = Fake::new("NV", ids(EVGA_PASCAL, "EVGA GeForce GTX 1080 FTW")).shared();
        let found = run(&[detect], &bus);
        assert_eq!(found.len(), 1);
        assert!(take().is_empty());
        let other = Fake::new("NV", [0x10DE, 0x1B80, 0x3842, 0x0000]).shared();
        assert!(run(&[detect], &other).is_empty());
    }

    // RGBController_EVGAGPUv1::DeviceUpdateMode (SetMode custom) then DeviceUpdateLEDs (SetColor).
    #[test]
    fn direct_and_frame() {
        let bus = Fake::new("NV", ids(EVGA_PASCAL, "EVGA GeForce GTX 1070 FTW")).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&["wr 49 0c 01", "wr 49 09 01", "wr 49 0a 02", "wr 49 0b 03"])
        );
        assert_eq!(
            push(&mut b, &[Color(4, 5, 6)]),
            s(&["wr 49 09 04", "wr 49 0a 05", "wr 49 0b 06"])
        );
    }
}
