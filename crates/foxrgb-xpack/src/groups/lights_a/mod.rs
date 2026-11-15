//! SignalRGB plugin ports, group lights_a (A4Tech through Gigabyte, ASUS peripherals and monitors).

mod asus_kb;
mod asus_monitor;
mod asus_mouse;
mod common;
mod misc_a;
mod misc_b;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    let mut v = asus_kb::detectors();
    v.extend(asus_mouse::detectors());
    v.extend(asus_monitor::detectors());
    v.extend(misc_a::detectors());
    v.extend(misc_b::detectors());
    v
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    misc_a::fan_ids()
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    misc_a::other_ids()
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    misc_a::probe_other()
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    misc_a::fan_backends(api)
}
