// SPDX-License-Identifier: GPL-2.0-or-later

use crate::registry::Detector;

mod common;
mod lenovo;
mod lenovo_gen7;
mod lenovo_zones;
mod lianli;
mod roccat;
mod sinowealth;
mod vulcan;
mod vulcan_layouts;

pub use lianli::{Reports, over_usb};

pub fn detectors() -> Vec<&'static Detector> {
    roccat::DETECTORS
        .iter()
        .chain(&vulcan::DETECTORS)
        .chain(&lenovo::DETECTORS)
        .chain(&lenovo_gen7::DETECTORS)
        .chain(&lianli::DETECTORS)
        .chain(&sinowealth::DETECTORS)
        .collect()
}
