//! Gigabyte AORUS Master / Xtreme Ampere cards.

use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_smbus::i2c::SharedBus;

use super::common::{Card, Driver, Gpu, Io, RECT, c, changed, detect_each, device_id};

const CMD_INIT: u8 = 0xD6;
const CMD_ZONE: u8 = 0xE6;
/// Every command is a 32-byte raw write.
const PACKET_LEN: usize = 32;
/// Zone id, then each LED's cell on the plugin's 14 x 6 canvas.
const ZONES: [(u8, &[(u8, u8)]); 5] = [
    (0x02, &[(0, 2), (0, 1), (0, 0)]),
    (0x03, &[(7, 4), (8, 4), (9, 4)]),
    (0x04, &[(13, 0), (13, 1), (13, 2)]),
    (0x05, &[(13, 5), (11, 5), (10, 5)]),
    (0x06, &[(7, 5), (8, 4), (9, 3), (10, 2), (11, 1), (11, 0)]),
];
const COLS: u32 = 14;
const ROWS: u32 = 6;

#[rustfmt::skip]
pub const GIGABYTE_XTREME: &[Card] = &[
    c("Gigabyte 3080 AORUS Master", 0x10DE, 0x2206, 0x1458, 0x403D, 0x60),
    c("Gigabyte 3080 AORUS Master", 0x10DE, 0x2206, 0x1458, 0x403E, 0x60),
    c("Gigabyte 3080 AORUS Master LHR", 0x10DE, 0x2216, 0x1458, 0x403D, 0x60),
    c("Gigabyte 3080 AORUS Master LHR", 0x10DE, 0x2216, 0x1458, 0x403E, 0x60),
    c("Gigabyte 3080 AORUS Master (GA102)", 0x10DE, 0x220A, 0x1458, 0x40A5, 0x60),
    c("Gigabyte 3080Ti XTREME", 0x10DE, 0x2208, 0x1458, 0x4080, 0x60),
    c("Gigabyte 3080Ti AORUS Master", 0x10DE, 0x2208, 0x1458, 0x4081, 0x60),
    c("Gigabyte 3090 XTREME", 0x10DE, 0x2204, 0x1458, 0x4041, 0x60),
];

#[cfg(test)]
fn leds() -> usize {
    ZONES.iter().map(|z| z.1.len()).sum()
}

// Scan has no probe: a matching card is taken as is.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, GIGABYTE_XTREME, |bus, card| {
        let id = device_id("gigabyte-xtreme", bus, card.addr);
        let mut dev = Device::grid(&id, card.name, DeviceKind::Gpu, RECT, COLS, ROWS);
        dev.leds = ZONES
            .iter()
            .flat_map(|z| z.1.iter())
            .map(|&(x, y)| {
                [
                    (x as f32 + 0.5) / COLS as f32,
                    (y as f32 + 0.5) / ROWS as f32,
                ]
            })
            .collect();
        Some(Gpu::boxed(bus, dev, Xtreme { addr: card.addr }))
    })
}

/// The packet for one zone: command, zone id, static mode, brightness 100, two fixed bytes, LED count, RGB.
fn zone_packet(zone: u8, colors: &[Color]) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[..7].copy_from_slice(&[CMD_ZONE, zone, 0x01, 0x64, 0x02, 0x03, colors.len() as u8]);
    for (i, c) in colors.iter().enumerate() {
        p[7 + i * 3..10 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    p
}

struct Xtreme {
    addr: u8,
}

impl Driver for Xtreme {
    fn setup(&mut self, io: &Io, _: &[Color]) {
        io.write_block(self.addr, CMD_INIT, &[0; PACKET_LEN - 1]);
    }

    // Raw writes equal block writes whose register is the first byte.
    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>) {
        let dirty = changed(colors, prev);
        let mut start = 0;
        for &(zone, cells) in &ZONES {
            let end = start + cells.len();
            if dirty.iter().any(|&i| (start..end).contains(&i)) {
                let p = zone_packet(zone, &colors[start..end]);
                io.write_block(self.addr, p[0], &p[1..]);
            }
            start = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, take};
    use super::*;

    #[test]
    fn init_then_zones() {
        let bus = Fake::new("NV", ids(GIGABYTE_XTREME, "Gigabyte 3090 XTREME")).shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert!(take().is_empty());
        assert_eq!(b.devices()[0].leds.len(), leds());
        let mut colors = vec![Color::BLACK; leds()];
        colors[0] = Color(1, 2, 3);
        let log = push(&mut b, &colors);
        assert_eq!(log.len(), 6);
        assert!(log[0].starts_with("wblk 60 d6 00 00"));
        assert!(log[1].starts_with("wblk 60 e6 02 01 64 02 03 03 01 02 03 00"));
        assert!(log[5].starts_with("wblk 60 e6 06 01 64 02 03 06 00"));
        colors[17] = Color(9, 9, 9);
        let log = push(&mut b, &colors);
        assert_eq!(log.len(), 1);
        assert!(log[0].starts_with("wblk 60 e6 06"));
    }

    #[test]
    fn packets_fit() {
        let p = zone_packet(0x06, &[Color(0xFF, 0, 0); 6]);
        assert_eq!(p[6], 6);
        assert_eq!(p[7..10], [0xFF, 0, 0]);
        assert_eq!(p[25..], [0; 7]);
    }
}
