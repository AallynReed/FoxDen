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

use foxden_hid::registry::Detector;

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
