//! Cooler Master, Acer, EVGA, EVision, Flydigi and Corsair Vanguard devices (protocol learned from SignalRGB's plugins of those brands).

mod coolermaster;
mod evga;
mod evision;
mod flydigi;
mod layouts;
mod vanguard;

use foxrgb_hid::registry::Detector;

pub fn detectors() -> Vec<&'static Detector> {
    coolermaster::DETECTORS
        .iter()
        .chain(evga::DETECTORS)
        .chain(evision::DETECTORS)
        .chain(flydigi::DETECTORS)
        .chain(vanguard::DETECTORS)
        .collect()
}
