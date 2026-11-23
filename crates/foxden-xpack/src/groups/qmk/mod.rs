//! QMK keyboards on SignalRGB firmware, SRGBmods LED controllers and pre-release Stream Decks.

mod boards;
mod jpeg;
mod keyboards;
mod srgbmods;
mod streamdeck;

use foxden_hid::registry::Detector;

pub fn detectors() -> Vec<&'static Detector> {
    boards::DETECTORS
        .iter()
        .chain(srgbmods::DETECTORS)
        .chain(streamdeck::DETECTORS)
        .collect()
}
