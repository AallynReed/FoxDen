// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::GALAX;
use super::common::{Driver, Gpu, Io, detect_each, device_id, row};

const REG_RED: u8 = 0x02;

const V1_MODE_1: u8 = 0x05;
const V1_MODE_2: u8 = 0x06;

const V2_SPEED_A: u8 = 0x21;
const V2_SPEED_B: u8 = 0x22;
const V2_SYNC: u8 = 0x27;
const V2_BRIGHTNESS: u8 = 0x2D;
const V2_MODE: u8 = 0x30;
const V2_MODE_STATIC: u8 = 0x01;
const V2_MODE_BREATHING: u8 = 0x02;
const V2_MODE_RAINBOW: u8 = 0x16;
const V2_MODE_OFF: u8 = 0x19;
const V2_SYNC_ON: u8 = 0x01;

// Probe; reads land in an unsigned char, so an error is 0xFF.
fn present(io: &Io, addr: u8) -> bool {
    let rd = |cmd| io.read(addr, cmd) as u8;
    match addr {
        0x32 => matches!(rd(0x00), 0x27 | 0x26) && matches!(rd(0x01), 0x10 | 0x20),
        0x23 => matches!(rd(0x00), 0x27 | 0x30),
        0x51 => rd(0x00) == 0x80,
        0x50 => rd(0x00) == 0x01 && rd(0x01) == 0x08,
        _ => false,
    }
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, GALAX, |bus, card| {
        let io = Io::new(bus);
        let addr = card.addr;
        if !present(&io, addr) {
            return None;
        }
        let dev = row(device_id("galax", bus, addr), card.name, 1);
        match addr {
            0x32 | 0x23 => Some(Gpu::boxed(bus, dev, V1 { addr })),
            _ => {
                // Mode, brightness and speed: a card already in Direct keeps them.
                let mode = io.read(addr, V2_MODE) as u8;
                let sync = io.read(addr, V2_SYNC) as u8;
                let brightness = io.read(addr, V2_BRIGHTNESS) as u8;
                let speed = io.read(addr, V2_SPEED_A) as u8;
                let direct = sync != V2_SYNC_ON
                    && !matches!(mode, V2_MODE_BREATHING | V2_MODE_RAINBOW | V2_MODE_OFF);
                let (brightness, speed) = if direct { (brightness, speed) } else { (0, 0) };
                Some(Gpu::boxed(
                    bus,
                    dev,
                    V2 {
                        addr,
                        brightness,
                        speed,
                    },
                ))
            }
        }
    })
}

fn set_colors(io: &Io, addr: u8, c: Color) {
    io.write(addr, REG_RED, c.0);
    io.write(addr, REG_RED + 1, c.1);
    io.write(addr, REG_RED + 2, c.2);
}

struct V1 {
    addr: u8,
}

impl Driver for V1 {
    // Mode 1: static.
    fn setup(&mut self, io: &Io, _: &[Color]) {
        io.write(self.addr, V1_MODE_1, 0x00);
        io.write(self.addr, V1_MODE_2, 0x01);
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        set_colors(io, self.addr, colors[0]);
    }
}

struct V2 {
    addr: u8,
    brightness: u8,
    speed: u8,
}

impl Driver for V2 {
    // Direct mode: sync off, static, speed, brightness.
    fn setup(&mut self, io: &Io, _: &[Color]) {
        io.write(self.addr, V2_SYNC, 0x00);
        io.write(self.addr, V2_MODE, V2_MODE_STATIC);
        io.write(self.addr, V2_SPEED_A, self.speed);
        io.write(self.addr, V2_SPEED_B, self.speed);
        io.write(self.addr, V2_BRIGHTNESS, self.brightness);
    }

    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        set_colors(io, self.addr, colors[0]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const V1_CARD: &str = "KFA2 GeForce RTX 2080 SUPER EX OC";
    const V2_CARD: &str = "KFA2 GeForce RTX 4080 SG 1-Click OC";
    const V2_5070: &str = "GALAX GeForce RTX 5070 Ti EX Gamer 1-Click OC";

    // Probe at 0x23.
    #[test]
    fn v1_detect() {
        let bus = Fake::new("NV", ids(GALAX, V1_CARD))
            .byte(0x23, 0x00, &[0x30])
            .shared();
        assert_eq!(run(&[detect], &bus).len(), 1);
        assert_eq!(take(), s(&["rd 23 00"]));
        let bus = Fake::new("NV", ids(GALAX, V1_CARD))
            .byte(0x23, 0x00, &[0x26])
            .shared();
        assert!(run(&[detect], &bus).is_empty());
    }

    // Direct mode and LEDs.
    #[test]
    fn v1_direct_and_frame() {
        let bus = Fake::new("NV", ids(GALAX, V1_CARD))
            .byte(0x23, 0x00, &[0x27])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&[
                "wr 23 05 00",
                "wr 23 06 01",
                "wr 23 02 01",
                "wr 23 03 02",
                "wr 23 04 03"
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(4, 5, 6)]),
            s(&["wr 23 02 04", "wr 23 03 05", "wr 23 04 06"])
        );
    }

    // Probe at 0x50 reads reg 0x01 only after reg 0x00 is 0x01.
    #[test]
    fn v2_detect_0x50() {
        let bus = Fake::new("NV", ids(GALAX, V2_CARD))
            .byte(0x50, 0x00, &[0x02])
            .shared();
        assert!(run(&[detect], &bus).is_empty());
        assert_eq!(take(), s(&["rd 50 00"]));
    }

    // v2 setup: device mode, brightness and speed, then the mode change.
    #[test]
    fn v2_direct_and_frame() {
        let bus = Fake::new("NV", ids(GALAX, V2_CARD))
            .byte(0x50, 0x00, &[0x01])
            .byte(0x50, 0x01, &[0x08])
            .byte(0x50, 0x30, &[0x01])
            .byte(0x50, 0x27, &[0x00])
            .byte(0x50, 0x2D, &[0x03])
            .byte(0x50, 0x21, &[0x05])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(
            take(),
            s(&[
                "rd 50 00", "rd 50 01", "rd 50 30", "rd 50 27", "rd 50 2d", "rd 50 21"
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&[
                "wr 50 27 00",
                "wr 50 30 01",
                "wr 50 21 05",
                "wr 50 22 05",
                "wr 50 2d 03",
                "wr 50 02 01",
                "wr 50 03 02",
                "wr 50 04 03",
            ])
        );
    }

    // v2 mode read: Rainbow leaves Direct at the mode() defaults.
    #[test]
    fn v2_defaults() {
        let bus = Fake::new("NV", ids(GALAX, V2_5070))
            .byte(0x51, 0x00, &[0x80])
            .byte(0x51, 0x30, &[0x16])
            .byte(0x51, 0x27, &[0x00])
            .byte(0x51, 0x2D, &[0x03])
            .byte(0x51, 0x21, &[0x05])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        let log = push(&mut b, &[Color(1, 2, 3)]);
        assert_eq!(log[2..5], s(&["wr 51 21 00", "wr 51 22 00", "wr 51 2d 00"]));
    }
}
