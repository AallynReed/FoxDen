// SPDX-License-Identifier: GPL-2.0-or-later
//! Device families, each in its own folder.

use crate::registry::{Detector, Matcher};
use foxden_core::Backend;

mod asus;
mod common;
mod coolermaster_corsair;
mod gigabyte_asrock;
mod logitech;
mod mixed_a;
mod mixed_b;
mod more_a;
mod more_b;
mod more_c;
mod more_d;
mod msi_steelseries;
mod razer;
mod small;
mod streamdeck;

/// Every detector of every ported family.
pub fn detectors() -> Vec<&'static Detector> {
    [
        razer::detectors(),
        logitech::detectors(),
        msi_steelseries::detectors(),
        coolermaster_corsair::detectors(),
        gigabyte_asrock::detectors(),
        mixed_a::detectors(),
        mixed_b::detectors(),
        small::detectors(),
        more_a::detectors(),
        more_b::detectors(),
        more_c::detectors(),
        more_d::detectors(),
        asus::detectors(),
        streamdeck::detectors(),
    ]
    .concat()
}

/// Ported devices that are not found through a HID collection: serial, WinUSB, SCSI and laptops.
pub fn other() -> Vec<Box<dyn Backend>> {
    let mut found = more_d::probe();
    found.extend(asus::others());
    found
}

pub use mixed_a::{Reports as UsbReports, over_usb as unihub_over_usb};
pub use more_d::{LedProtocol, LedStrip, led_strips, other_ids, serial_ports, winusb};

/// Vendor-wide `HID_PID_ANY` lines of the ported families.
pub fn matchers() -> Vec<&'static Matcher> {
    more_c::matchers()
}

#[cfg(test)]
mod tests {
    // Razer and Corsair ids belong to one family each.
    #[test]
    fn no_razer_or_corsair_id_in_two_families() {
        let families = [
            super::razer::detectors(),
            super::logitech::detectors(),
            super::msi_steelseries::detectors(),
            super::coolermaster_corsair::detectors(),
            super::gigabyte_asrock::detectors(),
            super::mixed_a::detectors(),
            super::mixed_b::detectors(),
            super::small::detectors(),
            super::more_a::detectors(),
            super::more_b::detectors(),
            super::more_c::detectors(),
            super::more_d::detectors(),
            super::asus::detectors(),
            super::streamdeck::detectors(),
        ];
        for (i, a) in families.iter().enumerate() {
            for b in &families[i + 1..] {
                for d in a.iter().filter(|d| matches!(d.vid, 0x1532 | 0x1B1C)) {
                    assert!(
                        !b.iter().any(|e| (e.vid, e.pid) == (d.vid, d.pid)),
                        "{} {:04x}:{:04x}",
                        d.name,
                        d.vid,
                        d.pid
                    );
                }
            }
        }
    }

    use std::collections::BTreeMap;

    /// Ids two drivers already share, told apart by interface or usage.
    const SHARED: [(u16, u16); 4] = [
        (0x0483, 0x5750),
        (0x05AC, 0x024F),
        (0x0B05, 0x1866),
        (0x1462, 0x3FA4),
    ];

    #[test]
    fn no_device_is_detected_by_two_drivers() {
        let mut by_id: BTreeMap<(u16, u16), Vec<usize>> = BTreeMap::new();
        for d in super::detectors() {
            let opens = by_id.entry((d.vid, d.pid)).or_default();
            let f = d.open as usize;
            if !opens.contains(&f) {
                opens.push(f);
            }
        }
        let twice: Vec<(u16, u16)> = by_id
            .into_iter()
            .filter(|(id, opens)| opens.len() > 1 && !SHARED.contains(id))
            .map(|(id, _)| id)
            .collect();
        assert!(twice.is_empty(), "{twice:04x?}");
    }
}
