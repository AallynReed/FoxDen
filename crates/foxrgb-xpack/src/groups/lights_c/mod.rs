//! lights_c: Razer misc, Roccat/Turtle Beach, SteelSeries, ThermalTake, SpeedLink and generic keyboard makers.

mod common;
mod creative;
mod kbm;
mod razer;
mod rk;
mod roccat;
mod sinowealth;
mod sonix;
mod sonix_cfg;
mod speedlink;
mod steelseries;
mod thermaltake;
mod xiaohua;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    [
        creative::DETECTORS,
        kbm::DETECTORS,
        razer::DETECTORS,
        rk::DETECTORS,
        roccat::DETECTORS,
        sinowealth::DETECTORS,
        sonix::DETECTORS,
        sonix_cfg::DETECTORS,
        speedlink::DETECTORS,
        steelseries::DETECTORS,
        thermaltake::DETECTORS,
        xiaohua::DETECTORS,
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    Vec::new()
}

pub fn fan_backends(_api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    Vec::new()
}
