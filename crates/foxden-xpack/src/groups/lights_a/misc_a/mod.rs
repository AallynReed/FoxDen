//! Alienware, Antec, A4Tech, ASRock, Attack Shark, Aula, BYTech, Dareu, Dark Project, Drevo and Gigabyte devices.

mod alienware;
mod antec;
mod keyboards;
mod layouts;
mod small;

use super::common::full;
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

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    antec::fan_backends(api)
}
