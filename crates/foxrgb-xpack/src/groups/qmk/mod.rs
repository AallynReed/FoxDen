//! QMK keyboards on SignalRGB firmware, SRGBmods LED controllers and pre-release Stream Decks.

mod boards;
mod jpeg;
mod keyboards;
mod srgbmods;
mod streamdeck;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    boards::DETECTORS
        .iter()
        .chain(srgbmods::DETECTORS)
        .chain(streamdeck::DETECTORS)
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
