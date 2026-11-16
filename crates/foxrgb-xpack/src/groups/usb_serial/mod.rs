//! Raw USB, USB serial and screen-kit devices: Aqua Computer, PCDOCK, Lian Li, Cooler
//! Master Atmos II Pixel and the Corsair XC7 ring. LCD image streaming is not ported.

pub(super) mod aqua;
mod atmos;
mod lianli_screen;
mod link;
mod pcdock;
mod xc7;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    let mut v = vec![&atmos::DETECTOR];
    v.extend(xc7::DETECTORS.iter());
    v
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    vec![(xc7::VID, xc7::PID, xc7::NAME)]
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v: Vec<_> = aqua::MODELS
        .iter()
        .filter(|m| m.pid != aqua::WITH_FANS)
        .map(|m| (aqua::VID, m.pid, m.name))
        .collect();
    v.push((pcdock::VID, pcdock::PID, pcdock::NAME));
    v.push((lianli_screen::VID, lianli_screen::PID, lianli_screen::NAME));
    v
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    let mut v = aqua::probe(|p| p != aqua::WITH_FANS);
    v.extend(lianli_screen::probe());
    v.extend(pcdock::probe());
    v
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    xc7::fan_backends(api)
}
