// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::MANLI;
use super::common::{Driver, Gpu, Io, c_string, detect_each, device_id, row};

const REG_RGB: u8 = 0xA0;
const MODE_STATIC: u8 = 0x00;
const STATIC_BRIGHTNESS: u8 = 100;
const VERSION_QUERY: [u8; 8] = [REG_RGB, 0xF1, 0, 0, 0, 0, 0, 0];
/// MANLI_GPU_CONFIG: controller versions with at least one zone.
const VERSIONS: [&str; 1] = ["N675M-1018"];

/// Version read.
fn version(io: &Io, addr: u8) -> Option<String> {
    if !io.raw_write(addr, &VERSION_QUERY) {
        return None;
    }
    io.raw_read(addr, 32).map(|d| c_string(&d))
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, MANLI, |bus, card| {
        let io = Io::new(bus);
        if !io.raw_write(card.addr, &VERSION_QUERY) || io.raw_read(card.addr, 32).is_none() {
            return None;
        }
        let v = version(&io, card.addr).unwrap_or_default();
        if !VERSIONS.contains(&v.as_str()) {
            return None;
        }
        let dev = row(device_id("manli", bus, card.addr), card.name, 1);
        Some(Gpu::boxed(bus, dev, Manli { addr: card.addr }))
    })
}

struct Manli {
    addr: u8,
}

impl Driver for Manli {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // Static mode, the closest it has to direct
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let Color(r, g, b) = colors[0];
        let mut pkt = [0u8; 30];
        pkt[0] = 0x01;
        pkt[6] = MODE_STATIC;
        pkt[7..10].copy_from_slice(&[r, g, b]);
        pkt[11] = STATIC_BRIGHTNESS;
        io.write_block(self.addr, REG_RGB, &pkt);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "MANLI GeForce RTX 4090 Gallardo";
    const QUERY: &str = "wraw 49 a0 f1 00 00 00 00 00 00";

    // Detection, then the version read; unknown versions are dropped.
    #[test]
    fn detect_version() {
        let bus = Fake::new("NV", ids(MANLI, CARD))
            .raw(0x49, &[b"N675M-1018\0"])
            .shared();
        assert_eq!(run(&[super::detect], &bus).len(), 1);
        assert_eq!(take(), s(&[QUERY, "rraw 49 32", QUERY, "rraw 49 32"]));
        let other = Fake::new("NV", ids(MANLI, CARD))
            .raw(0x49, &[b"N675E-1019\0"])
            .shared();
        assert!(run(&[super::detect], &other).is_empty());
    }

    // Command: on, mode, colour, speed 0 and brightness 100 at 0xA0.
    #[test]
    fn static_frame() {
        let bus = Fake::new("NV", ids(MANLI, CARD))
            .raw(0x49, &[b"N675M-1018\0"])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        let mut expect = String::from("wblk 49 a0 01 00 00 00 00 00 00 0a 0b 0c 00 64");
        expect.push_str(&" 00".repeat(18));
        assert_eq!(push(&mut b, &[Color(0x0A, 0x0B, 0x0C)]), vec![expect]);
    }
}
