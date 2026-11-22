// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::EVGA_GP102;
use super::common::{Driver, Gpu, Io, changed, detect_each, device_id, row};

const REG_MODE: u8 = 0x0C;
const REG_CMD: u8 = 0x0E;
const REG_VALID: u8 = 0x04;
const MODE_CUSTOM: u8 = 0x01;
const CMD_BEGIN: u8 = 0xE5;
const CMD_COLOR: u8 = 0xE9;
const CMD_END: u8 = 0xE0;

/// gpuzoneinfos (Nameplate, K|NGP|N backplate, FTW3 backplate): address, colour registers,
/// ready and clear responses.
struct Zone {
    addr: u8,
    color: [u8; 3],
    ready: i32,
    clear: i32,
}

const ZONES: [Zone; 3] = [
    Zone {
        addr: 0x4A,
        color: [0x09, 0x0A, 0x0B],
        ready: 0x03,
        clear: 0x00,
    },
    Zone {
        addr: 0x2A,
        color: [0x30, 0x31, 0x32],
        ready: 0xE9,
        clear: 0xE0,
    },
    Zone {
        addr: 0x4F,
        color: [0x30, 0x31, 0x32],
        ready: 0x03,
        clear: 0x00,
    },
];

fn is_valid(io: &Io, zone: &Zone) -> bool {
    (0..3).any(|_| matches!(io.read(zone.addr, REG_VALID) as u8, 0x1F | 0x91))
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, EVGA_GP102, |bus, card| {
        let io = Io::new(bus);
        let zones: Vec<&'static Zone> = ZONES.iter().filter(|z| is_valid(&io, z)).collect();
        if zones.is_empty() {
            return None;
        }
        let id = device_id("evga-gp102", bus, zones[0].addr);
        let dev = row(id, card.name, zones.len() as u32);
        Some(Gpu::boxed(bus, dev, Gp102 { zones }))
    })
}

struct Gp102 {
    zones: Vec<&'static Zone>,
}

impl Gp102 {
    // Colour write.
    fn set_color(io: &Io, z: &Zone, c: Color) {
        io.write(z.addr, REG_CMD, CMD_BEGIN);
        io.write(z.addr, REG_CMD, CMD_COLOR);
        if io.read(z.addr, REG_CMD) == z.ready {
            for (reg, v) in z.color.iter().zip([c.0, c.1, c.2]) {
                io.write(z.addr, *reg, v);
            }
            io.write(z.addr, REG_CMD, CMD_END);
            let _completed = io.read(z.addr, REG_CMD) == z.clear;
        }
    }
}

impl Driver for Gp102 {
    fn setup(&mut self, io: &Io, _: &[Color]) {
        for z in &self.zones {
            io.write(z.addr, REG_MODE, MODE_CUSTOM);
        }
    }

    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>) {
        for i in changed(colors, prev) {
            Self::set_color(io, self.zones[i], colors[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const KINGPIN: &str = "EVGA GeForce GTX 1080 Ti K|NGP|N";

    // Validity check: up to three reads per zone, stopping at 0x1F or 0x91.
    #[test]
    fn detect_zones() {
        let bus = Fake::new("NV", ids(EVGA_GP102, KINGPIN))
            .byte(0x4A, 0x04, &[0x00, 0x1F])
            .byte(0x2A, 0x04, &[0x91])
            .shared();
        let found = run(&[detect], &bus);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].leds.len(), 2);
        assert_eq!(
            take(),
            s(&[
                "rd 4a 04", "rd 4a 04", "rd 2a 04", "rd 4f 04", "rd 4f 04", "rd 4f 04",
            ])
        );
        let none = Fake::new("NV", ids(EVGA_GP102, KINGPIN)).shared();
        assert!(run(&[detect], &none).is_empty());
    }

    // Mode change, then the zones that changed.
    #[test]
    fn direct_and_frame() {
        let bus = Fake::new("NV", ids(EVGA_GP102, KINGPIN))
            .byte(0x4A, 0x04, &[0x1F])
            .byte(0x2A, 0x04, &[0x1F])
            .byte(0x4A, 0x0E, &[0x03, 0x00])
            .byte(0x2A, 0x0E, &[0xE9, 0xE0, 0x00])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        let log = push(&mut b, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(
            log,
            s(&[
                "wr 4a 0c 01",
                "wr 2a 0c 01",
                "wr 4a 0e e5",
                "wr 4a 0e e9",
                "rd 4a 0e",
                "wr 4a 09 01",
                "wr 4a 0a 02",
                "wr 4a 0b 03",
                "wr 4a 0e e0",
                "rd 4a 0e",
                "wr 2a 0e e5",
                "wr 2a 0e e9",
                "rd 2a 0e",
                "wr 2a 30 04",
                "wr 2a 31 05",
                "wr 2a 32 06",
                "wr 2a 0e e0",
                "rd 2a 0e",
            ])
        );
        // Only the backplate changed; its ack read now returns 0x00, so no colour is written.
        let log = push(&mut b, &[Color(1, 2, 3), Color(7, 8, 9)]);
        assert_eq!(log, s(&["wr 2a 0e e5", "wr 2a 0e e9", "rd 2a 0e"]));
    }
}
