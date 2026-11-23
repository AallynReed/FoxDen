// SPDX-License-Identifier: GPL-2.0-or-later

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

pub(crate) use common::Op;
pub(crate) use msi_board::{D99, hardware_185};

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
    // One detector per known id, plus the extra ids.
    #[test]
    fn detector_count() {
        assert_eq!(super::detectors().len(), 194 + 28 + 22);
    }
}
