// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::{SAPPHIRE_V1, SAPPHIRE_V3};
use super::common::{
    AMD_GPU_VEN, Card, Driver, Gpu, Io, detect_each, device_id, info, is_amd_gpu_i2c_bus, row,
};

const V1_REG_MODE: u8 = 0x00;
const V1_REG_BRIGHTNESS: u8 = 0x01;
const V1_REG_RED: u8 = 0x03;
const V1_MODE_CUSTOM: u8 = 0x04;
const V1_BRIGHTNESS_MAX: u8 = 0;

const V3_REG_MODE: u8 = 0x10;
const V3_REG_EXTERNAL_CONTROL: u8 = 0x0F;
const V3_REG_RED: u8 = 0x1A;
const V3_MODE_CUSTOM: u8 = 0x06;

// TestForSapphireGPUController.
fn present(bus: &SharedBus, card: &Card) -> bool {
    let i = info(bus);
    if i.pci_vendor == AMD_GPU_VEN && !is_amd_gpu_i2c_bus(&i) {
        return false;
    }
    Io::new(bus).read_byte(card.addr) >= 0
}

pub fn detect_v1(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, SAPPHIRE_V1, |bus, card| {
        if !present(bus, card) {
            return None;
        }
        // ReadConfiguration: the mode, then the active mode's brightness.
        let io = Io::new(bus);
        let mode = io.read(card.addr, V1_REG_MODE) as u8;
        let brightness = io.read(card.addr, V1_REG_BRIGHTNESS) as u8;
        let brightness = if mode == V1_MODE_CUSTOM {
            brightness
        } else {
            V1_BRIGHTNESS_MAX
        };
        let dev = row(device_id("sapphire-v1", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(
            bus,
            dev,
            V1 {
                addr: card.addr,
                brightness,
            },
        ))
    })
}

pub fn detect_v3(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, SAPPHIRE_V3, |bus, card| {
        present(bus, card).then(|| {
            let dev = row(device_id("sapphire-v3", bus, card.addr), card.name, 1);
            Gpu::boxed(bus, dev, V3 { addr: card.addr })
        })
    })
}

fn set_color(io: &Io, addr: u8, red: u8, c: Color) {
    io.write(addr, red, c.0);
    io.write(addr, red + 1, c.1);
    io.write(addr, red + 2, c.2);
}

struct V1 {
    addr: u8,
    brightness: u8,
}

impl Driver for V1 {
    // DeviceUpdateMode for Static: SetMode then SetBrightness.
    fn setup(&mut self, io: &Io, _: &[Color]) {
        io.write(self.addr, V1_REG_MODE, V1_MODE_CUSTOM);
        io.write(self.addr, V1_REG_BRIGHTNESS, self.brightness);
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        set_color(io, self.addr, V1_REG_RED, colors[0]);
    }
}

struct V3 {
    addr: u8,
}

impl Driver for V3 {
    // DeviceUpdateMode for Static: external control off, then the custom mode.
    fn setup(&mut self, io: &Io, _: &[Color]) {
        io.write(self.addr, V3_REG_EXTERNAL_CONTROL, 0);
        io.write(self.addr, V3_REG_MODE, V3_MODE_CUSTOM);
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        set_color(io, self.addr, V3_REG_RED, colors[0]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const V1_CARD: &str = "Sapphire Radeon RX Vega 56/64 Nitro+";
    const V3_CARD: &str = "Sapphire Radeon RX 7900 XTX Nitro+";

    // TestForSapphireGPUController: an AMD card needs the ADL bus, then a byte read.
    #[test]
    fn detect_needs_amd_bus() {
        let bus = Fake::new("NVIDIA", ids(SAPPHIRE_V3, V3_CARD))
            .plain(0x28, 0)
            .shared();
        assert!(run(&[detect_v1, detect_v3], &bus).is_empty());
        assert!(take().is_empty());
        let bus = Fake::new("AMD ADL", ids(SAPPHIRE_V3, V3_CARD)).shared();
        assert!(run(&[detect_v3], &bus).is_empty());
        assert_eq!(take(), s(&["rb 28"]));
    }

    // RGBController_SapphireNitroGlowV1: ReadConfiguration, DeviceUpdateMode, DeviceUpdateLEDs.
    #[test]
    fn v1_direct_and_frame() {
        let bus = Fake::new("AMD ADL", ids(SAPPHIRE_V1, V1_CARD))
            .plain(0x55, 0)
            .byte(0x55, 0x00, &[0x01])
            .byte(0x55, 0x01, &[0x02])
            .shared();
        let mut b = run(&[detect_v1, detect_v3], &bus).remove(0);
        assert_eq!(take(), s(&["rb 55", "rd 55 00", "rd 55 01"]));
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&[
                "wr 55 00 04",
                "wr 55 01 00",
                "wr 55 03 01",
                "wr 55 04 02",
                "wr 55 05 03",
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(4, 5, 6)]),
            s(&["wr 55 03 04", "wr 55 04 05", "wr 55 05 06"])
        );
    }

    // ReadConfiguration on a card already in Static keeps its brightness.
    #[test]
    fn v1_keeps_custom_brightness() {
        let bus = Fake::new("AMD ADL", ids(SAPPHIRE_V1, V1_CARD))
            .plain(0x55, 0)
            .byte(0x55, 0x00, &[0x04])
            .byte(0x55, 0x01, &[0x02])
            .shared();
        let mut b = run(&[detect_v1], &bus).remove(0);
        assert_eq!(push(&mut b, &[Color(1, 2, 3)])[1], "wr 55 01 02");
    }

    // RGBController_SapphireNitroGlowV3::DeviceUpdateMode (Static) then DeviceUpdateLEDs.
    #[test]
    fn v3_direct_and_frame() {
        let bus = Fake::new("AMD ADL", ids(SAPPHIRE_V3, V3_CARD))
            .plain(0x28, 0)
            .shared();
        let mut b = run(&[detect_v1, detect_v3], &bus).remove(0);
        assert_eq!(take(), s(&["rb 28"]));
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&[
                "wr 28 0f 00",
                "wr 28 10 06",
                "wr 28 1a 01",
                "wr 28 1b 02",
                "wr 28 1c 03",
            ])
        );
    }
}
