//! Driver groups, one folder per protocol family.

use foxden_core::Backend;
use foxden_fan::FanBackend;
use foxden_hid::Pick;
use foxden_hid::registry::Detector;
use hidapi::HidApi;
mod fans_a;
mod fans_b;
mod fans_c;
mod lights_a;
mod lights_b;
mod lights_c;
pub mod network;
mod qmk;
mod usb_serial;

pub fn detectors() -> Vec<&'static Detector> {
    let mut v = Vec::new();
    v.extend(fans_a::detectors());
    v.extend(fans_b::detectors());
    v.extend(fans_c::detectors());
    v.extend(lights_a::detectors());
    v.extend(lights_b::detectors());
    v.extend(lights_c::detectors());
    v.extend(usb_serial::detectors());
    v.extend(qmk::detectors());
    v
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v = Vec::new();
    v.extend(fans_a::fan_ids());
    v.extend(fans_b::fan_ids());
    v.extend(fans_c::fan_ids());
    v.extend(usb_serial::fan_ids());
    v
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v = Vec::new();
    v.extend(fans_c::other_ids());
    v.extend(usb_serial::other_ids());
    v
}

#[cfg(test)]
pub fn picked_other_ids() -> Vec<(u16, u16, &'static str)> {
    fans_c::other_ids()
}

/// Only fans_c has fan hubs among its raw-USB devices, so only it is picked from.
pub fn probe_other(pick: Pick) -> Vec<Box<dyn Backend>> {
    let mut v = fans_c::probe_other(pick);
    if matches!(pick, Pick::Only(_)) {
        return v;
    }
    v.extend(usb_serial::probe_other());
    v
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut v = Vec::new();
    v.extend(fans_a::fan_backends(api));
    v.extend(fans_b::fan_backends(api));
    v.extend(fans_c::fan_backends(api));
    v.extend(lights_a::misc_a::fan_backends(api));
    v.extend(usb_serial::fan_backends(api));
    v
}
