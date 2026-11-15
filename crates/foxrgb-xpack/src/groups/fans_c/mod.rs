//! Lian Li fan controllers and AIOs: Uni Fan hubs, Galahad II, HydroShift II (protocols learned from SignalRGB's Lian Li plugins).

mod galahad;
mod hydroshift;
mod unihub_hid;
mod unihub_usb;
mod usb;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::Pick;
use foxrgb_hid::registry::Detector;
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    galahad::DETECTORS.iter().collect()
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v = unihub_hid::ids();
    v.extend(galahad::ids());
    v.extend(
        unihub_usb::MODELS
            .iter()
            .map(|m| (unihub_usb::ENE, m.pid, m.name)),
    );
    v.push((hydroshift::NUVOTON, hydroshift::PID, hydroshift::NAME));
    v
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v: Vec<_> = unihub_usb::MODELS
        .iter()
        .map(|m| (unihub_usb::ENE, m.pid, m.name))
        .collect();
    v.push((hydroshift::NUVOTON, hydroshift::PID, hydroshift::NAME));
    v
}

pub fn probe_other(pick: Pick) -> Vec<Box<dyn Backend>> {
    let mut v = unihub_usb::lights(pick);
    if pick.wants(hydroshift::NUVOTON, hydroshift::PID) {
        v.extend(hydroshift::lights());
    }
    v
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut v = unihub_hid::fans(api);
    v.extend(galahad::fans(api));
    v.extend(unihub_usb::fans(api));
    v.extend(hydroshift::fans());
    v
}
