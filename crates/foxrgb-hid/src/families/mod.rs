//! Device families ported from OpenRGB, each in its own folder.

use crate::registry::{Detector, Matcher};
use foxrgb_core::Backend;

mod asus;
mod coolermaster_corsair;
mod generic;
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
        generic::detectors(),
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
