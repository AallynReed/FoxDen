//! Cooler Master, Acer, EVGA, EVision, Flydigi and Corsair Vanguard devices.

mod coolermaster;
mod evga;
mod evision;
mod flydigi;
mod layouts;
mod vanguard;

use foxden_hid::registry::Detector;

pub fn detectors() -> Vec<&'static Detector> {
    coolermaster::DETECTORS
        .iter()
        .chain(evga::DETECTORS)
        .chain(evision::DETECTORS)
        .chain(flydigi::DETECTORS)
        .chain(vanguard::DETECTORS)
        .collect()
}
