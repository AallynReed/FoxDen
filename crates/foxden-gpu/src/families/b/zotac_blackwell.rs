//! Ported from OpenRGB Controllers/ZotacBlackwellGPUController (GPL-2.0-or-later).

use std::time::Duration;

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::cards::ZOTAC_BLACKWELL;
use super::common::{Driver, Gpu, Io, changed, detect_each, device_id, info, row, sleep};

const REG_FIXED: u8 = 0x20;
const REG_COMMIT: u8 = 0x17;
const MODE_STATIC: u8 = 0x01;
const STATIC_BRIGHTNESS: u8 = 100;
const DELAY: Duration = Duration::from_micros(3000);
const COMMIT_DELAY: Duration = Duration::from_micros(10000);

/// device_zone_configs: device, subdevice and zone count.
const ZONE_CONFIGS: [(u16, u16, u32); 3] = [
    (0x2C02, 0x1762, 3),
    (0x2B85, 0x1761, 2),
    (0x2B87, 0x2761, 2),
];

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, ZOTAC_BLACKWELL, |bus, card| {
        if Io::new(bus).read(card.addr, 0x10) < 0 {
            return None;
        }
        let i = info(bus);
        let zones = ZONE_CONFIGS
            .iter()
            .find(|z| z.0 == i.pci_device && z.1 == i.pci_subsystem_device)
            .map_or(3, |z| z.2);
        let dev = row(
            device_id("zotac-blackwell", bus, card.addr),
            card.name,
            zones,
        );
        Some(Gpu::boxed(bus, dev, Blackwell { addr: card.addr }))
    })
}

struct Blackwell {
    addr: u8,
}

impl Blackwell {
    // DeviceUpdateZoneLEDs for Static: SetMode then Commit.
    fn zone(&self, io: &Io, zone: usize, c: Color) {
        let regs = [
            0x00,
            zone as u8,
            MODE_STATIC,
            c.0,
            c.1,
            c.2,
            0,
            0,
            0,
            STATIC_BRIGHTNESS,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        for (i, v) in regs.into_iter().enumerate() {
            io.write(self.addr, REG_FIXED + i as u8, v);
            sleep(DELAY);
        }
        io.write(self.addr, REG_COMMIT, 0x01);
        sleep(COMMIT_DELAY);
    }
}

impl Driver for Blackwell {
    fn setup(&mut self, io: &Io, colors: &[Color]) {
        self.update(io, colors, None);
    }

    fn setup_writes_leds(&self) -> bool {
        true
    }

    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>) {
        for z in changed(colors, prev) {
            self.zone(io, z, colors[z]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "ZOTAC GAMING GeForce RTX 5090 SOLID OC";

    fn zone_log(zone: u8, c: Color) -> Vec<String> {
        let regs = [0, zone, 1, c.0, c.1, c.2, 0, 0, 0, 100, 0, 0, 0, 0, 0, 0];
        let mut log = Vec::new();
        for (i, v) in regs.iter().enumerate() {
            log.push(format!("wr 4b {:02x} {v:02x}", 0x20 + i));
            log.push("sleep 3ms".into());
        }
        log.extend(s(&["wr 4b 17 01", "sleep 10ms"]));
        log
    }

    // DetectZotacBlackwellGPUControllersPCI: a byte read of 0x10, zones from the PCI ids.
    #[test]
    fn detect_probe() {
        let bus = Fake::new("NV", ids(ZOTAC_BLACKWELL, CARD))
            .byte(0x4B, 0x10, &[0])
            .shared();
        let found = run(&[super::detect], &bus);
        assert_eq!(found[0].devices()[0].leds.len(), 2);
        assert_eq!(take(), s(&["rd 4b 10"]));
        let absent = Fake::new("NV", ids(ZOTAC_BLACKWELL, CARD)).shared();
        assert!(run(&[super::detect], &absent).is_empty());
    }

    // RGBController_ZotacBlackwellGPU::DeviceUpdateMode in Static, then one changed zone.
    #[test]
    fn direct_and_frame() {
        let bus = Fake::new("NV", ids(ZOTAC_BLACKWELL, CARD))
            .byte(0x4B, 0x10, &[0])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        let (a, z) = (Color(1, 2, 3), Color(4, 5, 6));
        let mut expect = zone_log(0, a);
        expect.extend(zone_log(1, z));
        assert_eq!(push(&mut b, &[a, z]), expect);
        assert_eq!(
            push(&mut b, &[a, Color(7, 8, 9)]),
            zone_log(1, Color(7, 8, 9))
        );
    }
}
