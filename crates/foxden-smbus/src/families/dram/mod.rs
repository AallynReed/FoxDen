// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::Backend;

use crate::i2c::{Detect, SharedBus, lock};
use common::Io;
use spd::{DDR4, DDR5, Slot};

mod common;
mod corsair_vengeance;
mod crucial;
#[cfg(test)]
mod fake;
mod gigabyte_fusion2;
mod hyperx;
mod kingston_fury;
mod patriot_viper;
mod patriot_viper_steel;
mod spd;
mod tforce_xtreem;

type DramDetect = fn(&SharedBus, &Io, &mut [&mut Slot], &'static str) -> Vec<Box<dyn Backend>>;

/// DRAM detectors, in probe order.
const DRAM_DETECTORS: &[(&str, DramDetect, u16, u8)] = &[
    (
        "Corsair Vengeance RGB DRAM",
        corsair_vengeance::detect,
        spd::JEDEC_CORSAIR,
        DDR4,
    ),
    ("HyperX DRAM", hyperx::detect, spd::JEDEC_KINGSTON, DDR4),
    (
        "Kingston Fury DDR4 DRAM",
        kingston_fury::detect_ddr4,
        spd::JEDEC_KINGSTON,
        DDR4,
    ),
    (
        "Kingston Fury DDR4 DRAM",
        kingston_fury::detect_ddr4,
        spd::JEDEC_KINGSTON_2,
        DDR4,
    ),
    (
        "Kingston Fury DDR5 DRAM",
        kingston_fury::detect_ddr5,
        spd::JEDEC_KINGSTON,
        DDR5,
    ),
    (
        "Kingston Fury DDR5 DRAM",
        kingston_fury::detect_ddr5,
        spd::JEDEC_KINGSTON_2,
        DDR5,
    ),
    (
        "Kingston Fury DDR5 DRAM",
        kingston_fury::detect_ddr5,
        spd::JEDEC_KINGSTON_3,
        DDR5,
    ),
    (
        "Patriot Viper",
        patriot_viper::detect,
        spd::JEDEC_PATRIOT,
        DDR4,
    ),
    (
        "Patriot Viper Steel",
        patriot_viper_steel::detect,
        0xFF7E,
        DDR4,
    ),
    (
        "T-Force Xtreem DDR4 DRAM",
        tforce_xtreem::detect,
        spd::JEDEC_TEAMGROUP,
        DDR4,
    ),
];

/// BackgroundDetectI2CDRAMDevices for one bus: read SPD once, then run each matching row.
fn detect_dram(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    if !common::is_dram_smbus(lock(bus).info()) {
        return Vec::new();
    }
    let io = Io::new(bus);
    let (mut slots, dram_type) = spd::scan(&io);
    let mut found = Vec::new();
    for &(name, detect, jedec, ty) in DRAM_DETECTORS {
        if ty != dram_type || !slots.iter().any(|s| s.jedec == jedec) {
            continue;
        }
        let mut matching: Vec<&mut Slot> = slots.iter_mut().filter(|s| s.jedec == jedec).collect();
        found.extend(detect(bus, &io, &mut matching, name));
    }
    found
}

/// The I2C pass (Crucial, Gigabyte) runs before the DRAM pass.
pub fn all() -> Vec<Detect> {
    vec![crucial::detect, gigabyte_fusion2::detect, detect_dram]
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    // BackgroundDetectI2CDRAMDevices skips buses that IF_DRAM_SMBUS rejects
    #[test]
    fn other_buses_are_not_scanned() {
        let mut sim = Sim::default();
        sim.ddr4.insert(0x50, ddr4_spd([0x01, 0x98], &[]));
        let info = crate::i2c::BusInfo {
            pci_vendor: 0x10DE,
            ..Default::default()
        };
        let bus = bus_on(info, sim);
        for d in all() {
            assert!(d(&bus).is_empty());
        }
        assert!(take().is_empty());
    }

    // IF_DRAM_SMBUS accepts the AMD FCH and the listed Intel controllers only
    #[test]
    fn dram_smbus_ids() {
        let id = |v, d| crate::i2c::BusInfo {
            pci_vendor: v,
            pci_device: d,
            ..Default::default()
        };
        assert!(common::is_dram_smbus(&id(0x1022, 0x790B)));
        assert!(common::is_dram_smbus(&id(0x8086, 0x7F23)));
        assert!(!common::is_dram_smbus(&id(0x8086, 0x1234)));
        assert!(!common::is_dram_smbus(&id(0x1022, 0x7A23)));
    }
}
