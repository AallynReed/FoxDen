//! Group lights_a (A4Tech through Gigabyte, ASUS peripherals and monitors).

mod asus_kb;
mod asus_monitor;
mod asus_mouse;
mod common;
pub(super) mod misc_a;
mod misc_b;

use foxden_hid::registry::Detector;

pub fn detectors() -> Vec<&'static Detector> {
    let mut v = asus_kb::detectors();
    v.extend(asus_mouse::detectors());
    v.extend(asus_monitor::detectors());
    v.extend(misc_a::detectors());
    v.extend(misc_b::detectors());
    v
}

