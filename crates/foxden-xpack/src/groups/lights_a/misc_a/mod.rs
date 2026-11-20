//! Alienware, Antec, A4Tech, ASRock, Attack Shark, Aula, BYTech, Dareu, Dark Project, Drevo and Gigabyte devices (protocol learned from SignalRGB's plugins of those brands).

mod alienware;
mod antec;
mod keyboards;
mod layouts;
mod small;

use super::common::full;
use foxden_core::Backend;
use foxden_fan::FanBackend;
use foxden_hid::registry::Detector;
use hidapi::HidApi;

static ANTEC: &[Detector] = &[full(
    "Antec Vortex View 360",
    antec::VID,
    antec::PID,
    0,
    0xFF00,
    1,
    antec::open,
)];

pub fn detectors() -> Vec<&'static Detector> {
    alienware::DETECTORS
        .iter()
        .chain(small::DETECTORS)
        .chain(keyboards::DETECTORS)
        .chain(ANTEC)
        .collect()
}

/// The Antec AIO's sensor hub has no fan channels and its id is already a lighting detector.
pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    antec::fan_backends(api)
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    Vec::new()
}
