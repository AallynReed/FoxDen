//! Ported from OpenRGB Controllers/ (Corsair Hydro, EVGA USB, NZXT Hue+, HYTE, Arctic, JGINYUE V2, Seagate, Skydimo serial, BlinkyTape, LED strips) (GPL-2.0-or-later).

use crate::registry::Detector;
use foxrgb_core::Backend;

mod arctic;
mod blinkytape;
mod common;
mod corsair_hydro;
mod corsair_hydro2;
mod evga_keyboard;
mod evga_mouse;
mod evga_x12;
mod hyte_mousemat;
mod hyte_nexus;
mod jginyue_v2;
mod ledstrip;
mod nzxt_hue_plus;
mod scsi;
mod seagate;
mod serial;
mod skydimo_serial;
mod winusb;

pub use ledstrip::{LedProtocol, LedStrip, led_strips};

fn sets() -> [&'static [Detector]; 4] {
    [
        evga_keyboard::DETECTORS,
        evga_mouse::DETECTORS,
        evga_x12::DETECTORS,
        jginyue_v2::DETECTORS,
    ]
}

pub fn detectors() -> Vec<&'static Detector> {
    sets().into_iter().flatten().collect()
}

/// Serial, WinUSB and SCSI devices, which never show in the HID list. A port stays open while a
/// driver holds it, so a later detector cannot take it too.
pub fn probe() -> Vec<Box<dyn Backend>> {
    let ports = serial::usb_ports();
    let on = |ids: (u16, u16)| ports.iter().filter(move |p| (p.vid, p.pid) == ids);
    let mut found = Vec::new();
    // REGISTER_USB_SERIAL_DETECTOR lines
    found.extend(on(blinkytape::IDS).filter_map(blinkytape::open));
    for ids in hyte_mousemat::IDS {
        found.extend(on(ids).filter_map(hyte_mousemat::open));
    }
    for &(vid, pid, name) in hyte_nexus::DEVICES {
        found.extend(on((vid, pid)).filter_map(|p| hyte_nexus::open(p, name)));
    }
    found.extend(on(nzxt_hue_plus::IDS).filter_map(nzxt_hue_plus::open));
    found.extend(on(skydimo_serial::IDS).filter_map(skydimo_serial::open));
    // REGISTER_DETECTOR lines, after the serial ones as in DetectionManager
    found.extend(on(arctic::IDS).filter_map(arctic::open));
    found.extend(corsair_hydro::probe());
    found.extend(corsair_hydro2::probe());
    found.extend(seagate::probe());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // REGISTER_HID_DETECTOR* lines per Detect.cpp (JGINYUE V2 as registered on Windows)
    #[test]
    fn detector_counts_match_openrgb() {
        let counts: Vec<usize> = sets().iter().map(|s| s.len()).collect();
        assert_eq!(counts, [4, 2, 1, 1]);
        assert_eq!(detectors().len(), 8);
    }

    // Other detectors: serial (BlinkyTape, 2 CNVS, 2 Nexus, Hue+, Skydimo), Arctic, 3 Hydro, Hydro2
    #[test]
    fn other_device_counts() {
        assert_eq!(hyte_mousemat::IDS.len() + hyte_nexus::DEVICES.len(), 4);
        assert_eq!(corsair_hydro::DEVICES.len(), 3);
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
