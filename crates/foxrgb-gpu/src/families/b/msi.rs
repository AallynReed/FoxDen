//! Ported from OpenRGB Controllers/MSIGPUController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::{MSI_V1, MSI_V2};
use super::common::{
    AMD_GPU_VEN, Driver, Gpu, Io, detect_each, device_id, info, is_amd_gpu_i2c_bus, row, sleep,
};

const REG_BRIGHTNESS: u8 = 0x36;
const REG_SPEED: u8 = 0x38;
const REG_R1: u8 = 0x30;
const REG_G1: u8 = 0x31;
const REG_B1: u8 = 0x32;
const REG_MODE: u8 = 0x22;
const MODE_STATIC: u8 = 0x13;
const BRIGHTNESS_MAX: u8 = 5;
const BRIGHTNESS_MULTI: u8 = 20;

const V1_REG_UNKNOWN: u8 = 0x26;
/// RGBController_MSIGPU's mode values, Direct first.
const V1_MODES: [u8; 20] = [
    0x13, 0x08, 0x1A, 0x07, 0x05, 0x06, 0x15, 0x1F, 0x16, 0x0D, 0x0B, 0x09, 0x0F, 0x11, 0x1D, 0x14,
    0x04, 0x02, 0x03, 0x01,
];
/// R1 G1 B1 R2 G2 B2 R3 G3 B3, read by SetupZones.
const V1_COLOR_REGS: [u8; 9] = [0x30, 0x31, 0x32, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C];

const V2_REG_UNKNOWN: u8 = 0x2E;
const V2_MODE_IDLE: u8 = 0x1C;
const V2_DELAY: Duration = Duration::from_millis(20);

pub fn detect_v1(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    let i = info(bus);
    if i.pci_vendor == AMD_GPU_VEN && !is_amd_gpu_i2c_bus(&i) {
        return Vec::new();
    }
    detect_each(bus, MSI_V1, |bus, card| {
        let io = Io::new(bus);
        let reg = |r| io.read(card.addr, r) as u8;
        for r in V1_COLOR_REGS {
            reg(r);
        }
        // GetDeviceMode leaves Direct active when nothing matches, then the brightness is read into it.
        let mode = reg(REG_MODE);
        reg(REG_SPEED);
        let brightness = reg(REG_BRIGHTNESS) / BRIGHTNESS_MULTI;
        let direct = V1_MODES.iter().position(|&m| m == mode).unwrap_or(0) == 0;
        let brightness = if direct { brightness } else { BRIGHTNESS_MAX };
        let dev = row(device_id("msi", bus, card.addr), card.name, 1);
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

struct V1 {
    addr: u8,
    brightness: u8,
}

impl Driver for V1 {
    // RGBController_MSIGPU::DeviceUpdateMode
    fn setup(&mut self, io: &Io, _colors: &[Color]) {
        let value = self.brightness.wrapping_mul(BRIGHTNESS_MULTI);
        io.write(self.addr, REG_BRIGHTNESS, value);
        io.write(self.addr, REG_MODE, MODE_STATIC);
    }

    // RGBController_MSIGPU::DeviceUpdateLEDs: Direct writes colour 1 only
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let Color(r, g, b) = colors[0];
        io.write(self.addr, V1_REG_UNKNOWN, 0x00);
        io.write(self.addr, REG_R1, r);
        io.write(self.addr, REG_G1, g);
        io.write(self.addr, REG_B1, b);
    }
}

pub fn detect_v2(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, MSI_V2, |bus, card| {
        let dev = row(device_id("msi-v2", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(bus, dev, V2 { addr: card.addr }))
    })
}

struct V2 {
    addr: u8,
}

impl V2 {
    // MSIGPUv2Controller::MSIGPURegisterWrite
    fn write(&self, io: &Io, reg: u8, value: u8) {
        io.write(self.addr, reg, value);
        sleep(V2_DELAY);
    }
}

impl Driver for V2 {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // RGBController_MSIGPUv2::DeviceUpdateAll for Direct, used for mode and LED updates alike
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let Color(r, g, b) = colors[0];
        self.write(io, V2_REG_UNKNOWN, 0x00);
        self.write(io, REG_MODE, V2_MODE_IDLE);
        self.write(io, REG_R1, r);
        self.write(io, REG_G1, g);
        self.write(io, REG_B1, b);
        self.write(io, REG_BRIGHTNESS, BRIGHTNESS_MULTI * BRIGHTNESS_MAX);
        self.write(io, REG_MODE, MODE_STATIC);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const V1_CARD: &str = "MSI GeForce GTX 1070 Gaming X";

    // RGBController_MSIGPU constructor reads, then Direct: brightness and mode, then colour 1.
    #[test]
    fn v1_direct() {
        let bus = Fake::new("NV", ids(MSI_V1, V1_CARD))
            .byte(0x68, REG_MODE, &[MODE_STATIC])
            .byte(0x68, REG_BRIGHTNESS, &[60])
            .shared();
        let mut b = run(&[detect_v1], &bus).remove(0);
        assert_eq!(
            take(),
            s(&[
                "rd 68 30", "rd 68 31", "rd 68 32", "rd 68 27", "rd 68 28", "rd 68 29", "rd 68 2a",
                "rd 68 2b", "rd 68 2c", "rd 68 22", "rd 68 38", "rd 68 36",
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&[
                "wr 68 36 3c",
                "wr 68 22 13",
                "wr 68 26 00",
                "wr 68 30 01",
                "wr 68 31 02",
                "wr 68 32 03"
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(4, 5, 6)]),
            s(&["wr 68 26 00", "wr 68 30 04", "wr 68 31 05", "wr 68 32 06"])
        );
    }

    // A card in another effect keeps Direct at full brightness (5 * 20).
    #[test]
    fn v1_other_mode() {
        let bus = Fake::new("NV", ids(MSI_V1, V1_CARD))
            .byte(0x68, REG_MODE, &[0x08])
            .byte(0x68, REG_BRIGHTNESS, &[20])
            .shared();
        let mut b = run(&[detect_v1], &bus).remove(0);
        assert_eq!(push(&mut b, &[Color(1, 2, 3)])[0], "wr 68 36 64");
    }

    // RGBController_MSIGPUv2::DeviceUpdateAll, MSI_GPU_V2_MODE_STATIC, 20 ms after every write.
    #[test]
    fn v2_direct() {
        let card = MSI_V2[0].name;
        let bus = Fake::new("NV", ids(MSI_V2, card)).shared();
        let mut b = run(&[detect_v2], &bus).remove(0);
        let addr = MSI_V2[0].addr;
        let expect: Vec<String> = [
            (0x2E, 0),
            (0x22, 0x1C),
            (0x30, 7),
            (0x31, 8),
            (0x32, 9),
            (0x36, 100),
            (0x22, 0x13),
        ]
        .iter()
        .flat_map(|(r, v)| {
            [
                format!("wr {addr:02x} {r:02x} {v:02x}"),
                "sleep 20ms".into(),
            ]
        })
        .collect();
        assert_eq!(push(&mut b, &[Color(7, 8, 9)]), expect);
    }
}
