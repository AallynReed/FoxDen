//! Ported from OpenRGB Controllers/EVGATuringGPUController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::EVGA_TURING;
use super::common::{Driver, Gpu, Io, detect_each, device_id, row};

const REG_MODE: u8 = 0x60;
const REG_COLOR_A_RED: u8 = 0x6C;
const REG_COLOR_A_BRIGHTNESS: u8 = 0x6F;
const REG_COLOR_B_RED: u8 = 0x70;
const MODE_STATIC: u8 = 0x01;
const BRIGHTNESS_DEFAULT: u8 = 0x64;

// DetectEVGATuringGPUControllers has no probe. The constructor's mode read decides whether
// Direct keeps the card's brightness; the other readbacks only fill the UI.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, EVGA_TURING, |bus, card| {
        let io = Io::new(bus);
        let mut mode = io.read(card.addr, REG_MODE) as u8;
        if mode == 0xFF {
            mode = io.read(card.addr, REG_MODE) as u8;
        }
        let brightness = if mode == MODE_STATIC {
            io.read(card.addr, REG_COLOR_A_BRIGHTNESS) as u8
        } else {
            BRIGHTNESS_DEFAULT
        };
        let dev = row(device_id("evga-v2", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(
            bus,
            dev,
            V2 {
                addr: card.addr,
                brightness,
            },
        ))
    })
}

struct V2 {
    addr: u8,
    brightness: u8,
}

impl V2 {
    fn enable_write(&self, io: &Io, enable: bool) {
        if enable {
            for v in [0xE5, 0xE9, 0xF5, 0xF9] {
                io.write(self.addr, 0x0E, v);
            }
        } else {
            io.write(self.addr, 0x08, 0x01);
            io.write(self.addr, 0x0E, 0xF0);
            io.write(self.addr, 0x0E, 0xE0);
        }
    }

    fn send_color(&self, io: &Io, start: u8, c: Color) {
        io.write(self.addr, start, c.0);
        io.write(self.addr, start + 1, c.1);
        io.write(self.addr, start + 2, c.2);
        io.write(self.addr, start + 3, self.brightness);
    }
}

impl Driver for V2 {
    // SetMode(EVGA_GPU_V2_RGB_MODE_STATIC, colors[0], 0, speed, brightness).
    fn setup(&mut self, io: &Io, colors: &[Color]) {
        self.enable_write(io, true);
        io.write(self.addr, REG_MODE, MODE_STATIC);
        self.send_color(io, REG_COLOR_A_RED, colors[0]);
        self.enable_write(io, false);
    }

    // SetColor(colors[0], 0, brightness).
    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        self.enable_write(io, true);
        self.send_color(io, REG_COLOR_A_RED, colors[0]);
        self.send_color(io, REG_COLOR_B_RED, Color::BLACK);
        self.enable_write(io, false);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "EVGA GeForce RTX 2080 SUPER FTW3 Ultra";

    // RGBController_EVGAGPUv2 constructor: GetMode, and the card's brightness when it is static.
    #[test]
    fn detect_reads_mode() {
        let bus = Fake::new("NV", ids(EVGA_TURING, CARD))
            .byte(0x49, 0x60, &[0xFF, 0x01])
            .byte(0x49, 0x6F, &[0x30])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(take(), s(&["rd 49 60", "rd 49 60", "rd 49 6f"]));
        let log = push(&mut b, &[Color(1, 2, 3)]);
        assert!(log.contains(&"wr 49 6f 30".to_string()));
    }

    // EVGAGPUv2Controller::SetMode (static) then SetColor, default brightness 0x64.
    #[test]
    fn direct_and_frame() {
        let bus = Fake::new("NV", ids(EVGA_TURING, CARD))
            .byte(0x49, 0x60, &[0x0F])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(take(), s(&["rd 49 60"]));
        let enable = ["wr 49 0e e5", "wr 49 0e e9", "wr 49 0e f5", "wr 49 0e f9"];
        let disable = ["wr 49 08 01", "wr 49 0e f0", "wr 49 0e e0"];
        let mut expect: Vec<&str> = enable.to_vec();
        expect.extend([
            "wr 49 60 01",
            "wr 49 6c 01",
            "wr 49 6d 02",
            "wr 49 6e 03",
            "wr 49 6f 64",
        ]);
        expect.extend(disable);
        expect.extend(enable);
        expect.extend([
            "wr 49 6c 01",
            "wr 49 6d 02",
            "wr 49 6e 03",
            "wr 49 6f 64",
            "wr 49 70 00",
            "wr 49 71 00",
            "wr 49 72 00",
            "wr 49 73 64",
        ]);
        expect.extend(disable);
        assert_eq!(push(&mut b, &[Color(1, 2, 3)]), s(&expect));
        assert_eq!(push(&mut b, &[Color(9, 2, 3)]).len(), 15);
    }
}
