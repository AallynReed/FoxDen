//! Ported from OpenRGB Controllers (HyperX, MSI and Alienware keyboards, EVision, A4Tech, Holtek, QMK, Creative) (GPL-2.0-or-later).

use crate::registry::{Detector, Matcher};

mod aw3423dwf;
mod aw410k;
mod aw510k;
mod aw_monitor;
mod bloody_b820r;
mod bloody_mouse;
mod common;
mod creative_xg6;
mod evision;
mod evision_v2;
mod holtek_a070;
mod holtek_a1fa;
mod hyperx_alloy_elite;
mod hyperx_alloy_elite2;
mod hyperx_alloy_fps;
mod hyperx_eve1800;
mod hyperx_origins;
mod hyperx_origins2_65;
mod hyperx_origins_60_65;
mod hyperx_origins_core;
mod msi_clutch;
mod msi_kb1565;
mod msi_mystic64;
mod qmk_keychron;
mod qmk_vial;

fn sets() -> Vec<&'static [Detector]> {
    vec![
        hyperx_alloy_elite::DETECTORS,
        hyperx_alloy_elite2::DETECTORS,
        hyperx_alloy_fps::DETECTORS,
        hyperx_origins::DETECTORS,
        hyperx_origins_core::DETECTORS,
        hyperx_origins_60_65::DETECTORS,
        hyperx_eve1800::DETECTORS,
        hyperx_origins2_65::DETECTORS,
        msi_kb1565::DETECTORS,
        msi_mystic64::DETECTORS,
        msi_clutch::DETECTORS,
        aw410k::DETECTORS,
        aw510k::DETECTORS,
        aw3423dwf::DETECTORS,
        aw_monitor::DETECTORS,
        evision::DETECTORS,
        evision_v2::DETECTORS,
        bloody_b820r::DETECTORS,
        bloody_mouse::DETECTORS,
        holtek_a070::DETECTORS,
        holtek_a1fa::DETECTORS,
        qmk_vial::DETECTORS,
        creative_xg6::DETECTORS,
    ]
}

pub fn detectors() -> Vec<&'static Detector> {
    sets().into_iter().flatten().collect()
}

/// `HID_PID_ANY` lines.
pub fn matchers() -> Vec<&'static Matcher> {
    qmk_keychron::MATCHERS.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Windows REGISTER_HID_DETECTOR* lines per Detect.cpp
    #[test]
    fn detector_counts_match_openrgb() {
        let counts: Vec<usize> = sets().iter().map(|s| s.len()).collect();
        assert_eq!(
            counts,
            [
                1, 2, 1, 2, 2, 3, 1, 1, 1, 3, 1, 1, 1, 1, 1, 13, 2, 1, 5, 1, 1, 2, 1
            ]
        );
        assert_eq!(detectors().len(), 48);
        assert_eq!(matchers().len(), 1);
    }

    #[test]
    fn no_detector_clashes_with_another_family() {
        let all = crate::families::detectors();
        for a in detectors() {
            let same = all
                .iter()
                .filter(|b| {
                    a.vid == b.vid
                        && a.pid == b.pid
                        && a.interface == b.interface
                        && a.usage_page == b.usage_page
                        && a.usage == b.usage
                })
                .count();
            assert_eq!(same, 1, "{}", a.name);
        }
    }
}
