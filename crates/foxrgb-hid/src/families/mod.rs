//! Device families ported from OpenRGB, each in its own folder.

use crate::registry::{Detector, Matcher};

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
        generic::detectors(),
        streamdeck::detectors(),
    ]
    .concat()
}

/// Ported devices that are not HID collections: serial, WinUSB and SCSI.
pub fn other() -> Vec<Box<dyn foxrgb_core::Backend>> {
    more_d::probe()
}

pub use more_d::{LedProtocol, LedStrip, led_strips};

/// Vendor-wide `HID_PID_ANY` lines of the ported families.
pub fn matchers() -> Vec<&'static Matcher> {
    more_c::matchers()
}
