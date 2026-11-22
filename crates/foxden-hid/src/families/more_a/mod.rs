// SPDX-License-Identifier: GPL-2.0-or-later

use crate::registry::Detector;

mod annepro2;
mod common;
mod fnatic;
mod gaizhonggai;
mod klm;
mod mountain;
mod realtek;
mod realtek_argb;
mod realtek_bridge;
mod royuan;
mod sony;

fn sets() -> [&'static [Detector]; 8] {
    [
        &gaizhonggai::DETECTORS,
        &royuan::DETECTORS,
        &sony::DETECTORS,
        &annepro2::DETECTORS,
        &mountain::DETECTORS,
        &realtek_bridge::DETECTORS,
        &realtek_argb::DETECTORS,
        &fnatic::DETECTORS,
    ]
}

pub fn detectors() -> Vec<&'static Detector> {
    sets().into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // REGISTER_HID_DETECTOR* lines per Detect.cpp, plus SignalRGB's Royuan 0x4002 and miniStreak Speed
    #[test]
    fn detector_counts_match_openrgb() {
        let counts: Vec<usize> = sets().iter().map(|s| s.len()).collect();
        assert_eq!(counts, [9, 6, 5, 5, 3, 3, 1, 4]);
        assert_eq!(detectors().len(), 36);
    }

    #[test]
    fn no_duplicate_detectors() {
        let all = detectors();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let same = a.vid == b.vid
                    && a.pid == b.pid
                    && a.interface == b.interface
                    && a.usage_page == b.usage_page
                    && a.usage == b.usage;
                assert!(!same, "{} / {}", a.name, b.name);
            }
        }
    }
}
