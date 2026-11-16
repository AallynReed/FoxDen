//! Ported from OpenRGB Controllers/MSI*Controller and Controllers/SteelSeriesController (GPL-2.0-or-later).

use crate::registry::Detector;

mod common;
mod msi_3zone;
mod msi_board;
mod msi_board761;
mod msi_laptop;
mod msi_monitor;
mod msi_optix;
mod msi_vigor;
mod ss_apex;
mod ss_audio_mat;
mod ss_headset;
mod ss_rival;
mod ss_rival3;

pub fn detectors() -> Vec<&'static Detector> {
    [
        msi_board::detectors(),
        msi_laptop::detectors(),
        msi_monitor::detectors(),
        msi_optix::detectors(),
        msi_vigor::detectors(),
        msi_3zone::detectors(),
        ss_rival::detectors(),
        ss_rival3::detectors(),
        ss_audio_mat::detectors(),
        ss_apex::detectors(),
        ss_headset::detectors(),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    // One detector per REGISTER_HID_DETECTOR* line ported, plus the ids added from SignalRGB.
    #[test]
    fn detector_count() {
        assert_eq!(super::detectors().len(), 194 + 28 + 19);
    }
}
