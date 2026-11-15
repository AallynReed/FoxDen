//! Driver groups, one folder per protocol family.

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
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
    v.extend(lights_a::fan_ids());
    v.extend(lights_b::fan_ids());
    v.extend(lights_c::fan_ids());
    v.extend(usb_serial::fan_ids());
    v.extend(qmk::fan_ids());
    v
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v = Vec::new();
    v.extend(fans_a::other_ids());
    v.extend(fans_b::other_ids());
    v.extend(fans_c::other_ids());
    v.extend(lights_a::other_ids());
    v.extend(lights_b::other_ids());
    v.extend(lights_c::other_ids());
    v.extend(usb_serial::other_ids());
    v.extend(qmk::other_ids());
    v
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    let mut v = Vec::new();
    v.extend(fans_a::probe_other());
    v.extend(fans_b::probe_other());
    v.extend(fans_c::probe_other());
    v.extend(lights_a::probe_other());
    v.extend(lights_b::probe_other());
    v.extend(lights_c::probe_other());
    v.extend(usb_serial::probe_other());
    v.extend(qmk::probe_other());
    v
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut v = Vec::new();
    v.extend(fans_a::fan_backends(api));
    v.extend(fans_b::fan_backends(api));
    v.extend(fans_c::fan_backends(api));
    v.extend(lights_a::fan_backends(api));
    v.extend(lights_b::fan_backends(api));
    v.extend(lights_c::fan_backends(api));
    v.extend(usb_serial::fan_backends(api));
    v.extend(qmk::fan_backends(api));
    v
}
