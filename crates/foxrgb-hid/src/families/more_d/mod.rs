//! Ported from OpenRGB Controllers/ (Corsair Hydro, Dygma Raise, EVGA USB, NZXT Hue+, HYTE, Arctic, JGINYUE V2, Seagate, Skydimo serial, Thrustmaster Sol, BlinkyTape, LED strips) (GPL-2.0-or-later).

use std::sync::Mutex;

use crate::registry::Detector;
use foxrgb_core::Backend;
use serial::Port;

mod arctic;
mod blinkytape;
mod common;
mod corsair_hydro;
mod corsair_hydro2;
mod dygma_raise;
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
mod thrustmaster_sol;
pub mod winusb;

pub use ledstrip::{LedProtocol, LedStrip, led_strips};

/// COM ports of USB devices, for picking a strip's port.
pub fn serial_ports() -> Vec<String> {
    let mut names: Vec<String> = serial::usb_ports().into_iter().map(|p| p.name).collect();
    names.sort_by_key(|n| (n[3..].parse::<u32>().unwrap_or(u32::MAX), n.clone()));
    names.dedup();
    names
}

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
/// Ports on generic USB-serial chips that did not answer as ours, skipped until unplugged:
/// asking again can block for seconds and DTR can reset the board on them.
static NOT_OURS: Mutex<Vec<(String, u16, u16)>> = Mutex::new(Vec::new());

fn key(p: &Port) -> (String, u16, u16) {
    (p.name.clone(), p.vid, p.pid)
}

/// Opens generic-chip ports once, remembering the ones that are something else.
fn once(
    ports: &[Port],
    ids: (u16, u16),
    open: fn(&Port) -> Option<Box<dyn Backend>>,
) -> Vec<Box<dyn Backend>> {
    let mut skip = NOT_OURS.lock().unwrap_or_else(|e| e.into_inner());
    let mut found = Vec::new();
    for p in ports.iter().filter(|p| (p.vid, p.pid) == ids) {
        if skip.contains(&key(p)) {
            continue;
        }
        match open(p) {
            Some(dev) => found.push(dev),
            None => skip.push(key(p)),
        }
    }
    found
}

/// Ids of the serial and WinUSB devices `probe` opens.
pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v = vec![
        (blinkytape::IDS.0, blinkytape::IDS.1, "BlinkyTape"),
        (nzxt_hue_plus::IDS.0, nzxt_hue_plus::IDS.1, "NZXT Hue+"),
        (
            skydimo_serial::IDS.0,
            skydimo_serial::IDS.1,
            "Skydimo / Arctic",
        ),
        (dygma_raise::IDS.0, dygma_raise::IDS.1, "Dygma Raise"),
        (0x1B1C, 0x0C09, "Corsair Hydro"),
    ];
    v.extend(hyte_mousemat::IDS.iter().map(|&(a, b)| (a, b, "HYTE CNVS")));
    v.extend(hyte_nexus::DEVICES.iter().copied());
    v.extend(corsair_hydro::DEVICES.iter().map(|&(p, n)| (0x1B1C, p, n)));
    v.extend(
        thrustmaster_sol::DEVICES
            .iter()
            .map(|&(p, n)| (0x044F, p, n)),
    );
    v
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    let ports = serial::usb_ports();
    NOT_OURS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|k| ports.iter().any(|p| key(p) == *k));
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
    found.extend(once(&ports, skydimo_serial::IDS, skydimo_serial::open));
    // REGISTER_DETECTOR lines, after the serial ones as in DetectionManager
    found.extend(once(&ports, arctic::IDS, arctic::open));
    found.extend(corsair_hydro::probe());
    found.extend(corsair_hydro2::probe());
    found.extend(on(dygma_raise::IDS).filter_map(dygma_raise::open));
    found.extend(seagate::probe());
    found.extend(thrustmaster_sol::probe());
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

    // Other detectors: serial (BlinkyTape, 2 CNVS, 2 Nexus, Hue+, Skydimo), Arctic, 3 Hydro, Hydro2, Dygma, 6 Sol
    #[test]
    fn other_device_counts() {
        assert_eq!(hyte_mousemat::IDS.len() + hyte_nexus::DEVICES.len(), 4);
        assert_eq!(corsair_hydro::DEVICES.len(), 3);
        assert_eq!(thrustmaster_sol::DEVICES.len(), 6);
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
