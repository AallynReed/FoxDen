// SPDX-License-Identifier: GPL-2.0-or-later

use crate::registry::Detector;
use foxden_core::Backend;

mod ally;
mod aura_core;
mod aura_core_laptop;
mod aura_keyboard;
mod aura_monitor;
mod aura_mouse;
mod aura_usb;
mod common;
mod coolers;
mod dmi;
mod headset_stand;
mod kb_layouts;
mod laptop_layouts;
mod legacy;
mod mouse_gen1;
mod mousemat;
mod tuf_laptop;

use super::small::klm;

fn sets() -> [&'static [Detector]; 12] {
    [
        aura_usb::DETECTORS,
        aura_monitor::DETECTORS,
        headset_stand::DETECTORS,
        mousemat::DETECTORS,
        coolers::DETECTORS,
        ally::DETECTORS,
        aura_mouse::DETECTORS,
        mouse_gen1::DETECTORS,
        aura_keyboard::DETECTORS,
        legacy::DETECTORS,
        // 0x1866's 0xFF31 collection goes to the laptop driver before Aura Core's catch-all
        aura_core_laptop::DETECTORS,
        aura_core::DETECTORS,
    ]
}

pub fn detectors() -> Vec<&'static Detector> {
    sets().into_iter().flatten().collect()
}

/// Lighting that is not on USB HID: the TUF laptop keyboard through ATKACPI.
pub fn others() -> Vec<Box<dyn Backend>> {
    tuf_laptop::probe().into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // REGISTER_HID_DETECTOR* lines per Detect.cpp; the Spatha X Dock shares the 2.4GHz mouse's line
    #[test]
    fn detector_counts_match_openrgb() {
        let counts: Vec<usize> = sets().iter().map(|s| s.len()).collect();
        assert_eq!(counts, [11, 6, 3, 2, 3, 2, 43, 3, 29, 3, 2, 3]);
        assert_eq!(detectors().len(), 109 - 1 + 2);
    }

    #[test]
    fn no_duplicate_detectors() {
        let all = detectors();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let same = a.vid == b.vid
                    && a.pid == b.pid
                    && a.interface == b.interface
                    && a.usage_page == b.usage_page
                    && a.usage == b.usage;
                assert!(!same, "{} / {}", a.name, b.name);
            }
        }
    }
}
