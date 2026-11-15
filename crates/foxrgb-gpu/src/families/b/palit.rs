//! Ported from OpenRGB Controllers/PalitGPUController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::{PALIT_V1, PALIT_V2};
use super::common::{Card, Driver, Gpu, Io, detect_each, device_id, row};

const V1_RED: u8 = 0x03;

const V2_CONTROL: u8 = 0x60;
const V2_RED: u8 = 0x6C;
const V2_GREEN: u8 = 0x6D;
const V2_BLUE: u8 = 0x6E;
const V2_BRIGHTNESS: u8 = 0x6F;
const V2_MODE: u8 = 0xE0;
const V2_SPEED: u8 = 0xE2;
const V2_STATIC_SOFTWARE: u8 = 0x01;

pub fn detect_v1(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, PALIT_V1, probe_v1)
}

pub fn detect_v2(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, PALIT_V2, probe_v2)
}

fn probe_v1(bus: &SharedBus, card: &Card) -> Option<Box<dyn Backend>> {
    let io = Io::new(bus);
    let mut matched = true;
    for (i, &letter) in b"PALIT".iter().enumerate() {
        if io.read(card.addr, 0x07 + i as u8) != letter as i32 {
            matched = false;
        }
    }
    // OpenRGB tests the s32 result of the quick write as a bool, so only a failed write passes.
    let pass = matched || (card.addr == 0x08 && !io.quick_ok(card.addr));
    pass.then(|| {
        let dev = row(device_id("palit-v1", bus, card.addr), card.name, 1);
        Gpu::boxed(bus, dev, V1 { addr: card.addr })
    })
}

fn probe_v2(bus: &SharedBus, card: &Card) -> Option<Box<dyn Backend>> {
    if card.addr != 0x49 {
        return None;
    }
    let io = Io::new(bus);
    let data = io.read(card.addr, 0x00);
    let mode = io.read(card.addr, V2_MODE);
    (data == 0 && mode < 5).then(|| {
        let dev = row(device_id("palit-v2", bus, card.addr), card.name, 1);
        Gpu::boxed(bus, dev, V2 { addr: card.addr })
    })
}

struct V1 {
    addr: u8,
}

impl Driver for V1 {
    fn setup(&mut self, _: &Io, _: &[Color]) {}

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        for c in colors {
            io.write(self.addr, V1_RED, c.0);
            io.write(self.addr, V1_RED + 1, c.1);
            io.write(self.addr, V1_RED + 2, c.2);
            io.write(self.addr, V1_RED + 3, 0xFF);
        }
    }
}

struct V2 {
    addr: u8,
}

impl Driver for V2 {
    // SetDirect with the Direct mode's default brightness of 255.
    fn setup(&mut self, io: &Io, colors: &[Color]) {
        let c = colors[0];
        io.write(self.addr, V2_MODE, 0);
        io.write(self.addr, V2_CONTROL, 0x01);
        io.write(self.addr, V2_RED, c.0);
        io.write(self.addr, V2_BLUE, c.2);
        io.write(self.addr, V2_GREEN, c.1);
        io.write(self.addr, V2_BRIGHTNESS, 255);
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        for c in colors {
            io.write(self.addr, V2_RED, c.0);
            io.write(self.addr, V2_GREEN, c.1);
            io.write(self.addr, V2_BLUE, c.2);
            io.write(self.addr, V2_MODE, 0x00);
            io.write(self.addr, V2_CONTROL, V2_STATIC_SOFTWARE);
            io.write(self.addr, V2_SPEED, 0x02);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, push, run, s, take};
    use super::*;

    const GTX1080: [u16; 4] = [0x10DE, 0x1B80, 0x10DE, 0x1B80];
    const GAINWARD_1080: [u16; 4] = [0x10DE, 0x1B80, 0x10B0, 0x1B80];
    const RTX3080_PALIT: [u16; 4] = [0x10DE, 0x2206, 0x1569, 0x2206];

    fn palit_string(f: Fake) -> Fake {
        b"PALIT"
            .iter()
            .enumerate()
            .fold(f, |f, (i, &l)| f.byte(0x08, 0x07 + i as u8, &[l]))
    }

    // TestForPalitGPUv1Controller: reads all five letters, no quick write once they match.
    #[test]
    fn v1_detect_by_string() {
        let bus = palit_string(Fake::new("NV", GTX1080)).shared();
        let found = run(&[detect_v1, detect_v2], &bus);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].name, "Palit GeForce GTX 1080");
        assert_eq!(
            take(),
            s(&["rd 08 07", "rd 08 08", "rd 08 09", "rd 08 0a", "rd 08 0b"])
        );
    }

    // TestForPalitGPUv1Controller: the Gainward fallback passes only when the quick write fails.
    #[test]
    fn v1_gainward_quick_fallback() {
        let bus = Fake::new("NV", GAINWARD_1080).shared();
        assert_eq!(run(&[detect_v1], &bus).len(), 1);
        assert_eq!(
            take(),
            s(&[
                "rd 08 07", "rd 08 08", "rd 08 09", "rd 08 0a", "rd 08 0b", "quick 08"
            ])
        );
        let mut ok = Fake::new("NV", GAINWARD_1080);
        ok.quick_ok = true;
        assert!(run(&[detect_v1], &ok.shared()).is_empty());
    }

    // RGBController_PalitGPUv1::DeviceUpdateLEDs -> SetLEDColors.
    #[test]
    fn v1_frame() {
        let bus = palit_string(Fake::new("NV", GTX1080)).shared();
        let mut b = run(&[detect_v1], &bus).remove(0);
        let log = push(&mut b, &[Color(1, 2, 3)]);
        assert_eq!(
            log,
            s(&["wr 08 03 01", "wr 08 04 02", "wr 08 05 03", "wr 08 06 ff"])
        );
        assert!(push(&mut b, &[Color(1, 2, 3)]).is_empty());
    }

    // TestForPalitGPUv2Controller: reg 0x00 must be 0 and the mode below 5.
    #[test]
    fn v2_detect() {
        let bus = Fake::new("NV", RTX3080_PALIT)
            .byte(0x49, 0x00, &[0])
            .byte(0x49, 0xE0, &[4])
            .shared();
        assert_eq!(run(&[detect_v1, detect_v2], &bus).len(), 1);
        assert_eq!(take(), s(&["rd 49 00", "rd 49 e0"]));
        let bad = Fake::new("NV", RTX3080_PALIT)
            .byte(0x49, 0x00, &[0])
            .byte(0x49, 0xE0, &[5]);
        assert!(run(&[detect_v2], &bad.shared()).is_empty());
        // A failed mode read is -1, which is below 5.
        let failed = Fake::new("NV", RTX3080_PALIT).byte(0x49, 0x00, &[0]);
        assert_eq!(run(&[detect_v2], &failed.shared()).len(), 1);
    }

    // RGBController_PalitGPUv2::DeviceUpdateMode (SetDirect) then DeviceUpdateLEDs.
    #[test]
    fn v2_direct_and_frame() {
        let bus = Fake::new("NV", RTX3080_PALIT)
            .byte(0x49, 0x00, &[0])
            .byte(0x49, 0xE0, &[0])
            .shared();
        let mut b = run(&[detect_v2], &bus).remove(0);
        let log = push(&mut b, &[Color(0x10, 0x20, 0x30)]);
        assert_eq!(
            log,
            s(&[
                "wr 49 e0 00",
                "wr 49 60 01",
                "wr 49 6c 10",
                "wr 49 6e 30",
                "wr 49 6d 20",
                "wr 49 6f ff",
                "wr 49 6c 10",
                "wr 49 6d 20",
                "wr 49 6e 30",
                "wr 49 e0 00",
                "wr 49 60 01",
                "wr 49 e2 02",
            ])
        );
        let log = push(&mut b, &[Color(0xAA, 0xBB, 0xCC)]);
        assert_eq!(
            log,
            s(&[
                "wr 49 6c aa",
                "wr 49 6d bb",
                "wr 49 6e cc",
                "wr 49 e0 00",
                "wr 49 60 01",
                "wr 49 e2 02",
            ])
        );
    }

    // DetectionManager::BackgroundDetectI2CPCIDevices: all four PCI ids must match first.
    #[test]
    fn other_cards_ignored() {
        let bus = palit_string(Fake::new("NV", [0x10DE, 0x1B80, 0x10DE, 0x0001])).shared();
        assert!(run(&[detect_v1, detect_v2], &bus).is_empty());
        assert!(take().is_empty());
    }
}
