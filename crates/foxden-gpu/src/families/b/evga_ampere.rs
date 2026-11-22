// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::EVGA_AMPERE;
use super::common::{Driver, Gpu, Io, changed, detect_each, device_id, row};

const REG_FIRMWARE: u8 = 0xB1;
const REG_ENABLE: u8 = 0xB2;
const REG_MODE: u8 = 0xC0;
const REG_STATIC: u8 = 0xC1;
const MODE_STATIC: u8 = 0x01;
const INIT: u8 = 0xFF;
const BRIGHTNESS_DEFAULT: u8 = 0xFF;

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, EVGA_AMPERE, |bus, card| {
        let io = Io::new(bus);
        let addr = card.addr;
        // ReadFWVersion: a full 6 byte read or no device.
        io.read_block(addr, REG_FIRMWARE, 6)?;
        // GetDeviceModes, then initCard.
        let (modes, counts) = match io.read_block(addr, REG_MODE, 10) {
            Some(p) => ([p[1], p[2], p[3], p[4]], [p[5], p[6], p[7], p[8]]),
            None => ([0; 4], [0; 4]),
        };
        init_card(&io, addr);
        let zones: Vec<u8> = (0..4u8).filter(|&z| counts[z as usize] > 0).collect();
        // The constructor loads the active mode's settings per zone; for Static they set the
        // brightness Direct then uses.
        let mut brightness = BRIGHTNESS_DEFAULT;
        if modes[0] == MODE_STATIC {
            for &z in &zones {
                if let Some(p) = io.read_block(addr, REG_STATIC + z, 5) {
                    brightness = p[1];
                } else {
                    brightness = BRIGHTNESS_DEFAULT;
                }
            }
        }
        if zones.is_empty() {
            return None;
        }
        let dev = row(
            device_id("evga-v3", bus, addr),
            card.name,
            zones.len() as u32,
        );
        Some(Gpu::boxed(
            bus,
            dev,
            V3 {
                addr,
                zones,
                brightness,
            },
        ))
    })
}

fn init_card(io: &Io, addr: u8) {
    io.write_block(addr, REG_ENABLE, &[0x04, 0xC6, 0xEB, 0xEA, 0x15]);
}

struct V3 {
    addr: u8,
    zones: Vec<u8>,
    brightness: u8,
}

impl V3 {
    // SetZone for EVGA_GPU_V3_MODE_STATIC.
    fn set_zone(&self, io: &Io, zone: u8, c: Color) {
        io.write_block(
            self.addr,
            REG_STATIC + zone,
            &[4, self.brightness, c.0, c.1, c.2],
        );
    }
}

impl Driver for V3 {
    // Mode change: the LEDs, then every mode set to static x4, synced.
    fn setup(&mut self, io: &Io, colors: &[Color]) {
        self.update(io, colors, None);
        init_card(io, self.addr);
        let m = MODE_STATIC;
        io.write_block(
            self.addr,
            REG_MODE,
            &[9, m, m, m, m, INIT, INIT, INIT, INIT, 1],
        );
    }

    fn setup_writes_leds(&self) -> bool {
        true
    }

    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>) {
        for i in changed(colors, prev) {
            self.set_zone(io, self.zones[i], colors[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "EVGA GeForce RTX 3080 FTW3 Ultra";

    fn card() -> Fake {
        Fake::new("NV", ids(EVGA_AMPERE, CARD)).block(0x2D, 0xB1, &[6, 0, 0, 0, 1, 2])
    }

    // No firmware reply, no device.
    #[test]
    fn detect_needs_firmware() {
        let bus = Fake::new("NV", ids(EVGA_AMPERE, CARD))
            .block(0x2D, 0xB1, &[6, 0, 0])
            .shared();
        assert!(run(&[detect], &bus).is_empty());
        assert_eq!(take(), s(&["rblk 2d b1 6"]));
    }

    // ReadFWVersion, GetDeviceModes, initCard, then GetZoneConfig for a static card.
    #[test]
    fn detect_zones() {
        let bus = card()
            .block(0x2D, 0xC0, &[9, 1, 1, 1, 1, 4, 0, 3, 0, 1])
            .block(0x2D, 0xC1, &[4, 0x80, 1, 2, 3])
            .block(0x2D, 0xC3, &[4, 0x40, 1, 2, 3])
            .shared();
        let found = run(&[detect], &bus);
        assert_eq!(found[0].devices()[0].leds.len(), 2);
        assert_eq!(
            take(),
            s(&[
                "rblk 2d b1 6",
                "rblk 2d c0 10",
                "wblk 2d b2 04 c6 eb ea 15",
                "rblk 2d c1 5",
                "rblk 2d c3 5",
            ])
        );
        let mut b = found.into_iter().next().unwrap();
        let log = push(&mut b, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(log[0], "wblk 2d c1 04 40 01 02 03");
    }

    // Mode change, then each zone's colour.
    #[test]
    fn direct_and_frame() {
        let bus = card()
            .block(0x2D, 0xC0, &[9, 2, 2, 2, 2, 4, 0, 0, 30, 1])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(
            take(),
            s(&["rblk 2d b1 6", "rblk 2d c0 10", "wblk 2d b2 04 c6 eb ea 15"])
        );
        let log = push(&mut b, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(
            log,
            s(&[
                "wblk 2d c1 04 ff 01 02 03",
                "wblk 2d c4 04 ff 04 05 06",
                "wblk 2d b2 04 c6 eb ea 15",
                "wblk 2d c0 09 01 01 01 01 ff ff ff ff 01",
            ])
        );
        let log = push(&mut b, &[Color(1, 2, 3), Color(7, 8, 9)]);
        assert_eq!(log, s(&["wblk 2d c4 04 ff 07 08 09"]));
    }

    // GetDeviceModes: a short read leaves every zone at zero LEDs.
    #[test]
    fn no_zones_no_device() {
        let bus = card().shared();
        assert!(run(&[detect], &bus).is_empty());
    }
}
