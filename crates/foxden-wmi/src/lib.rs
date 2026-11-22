// SPDX-License-Identifier: GPL-2.0-or-later
//! Laptop lighting and fan speeds through vendor WMI methods. Needs administrator rights, or
//! the installed service.

mod hp_omen;
mod hp_omen_fans;

use foxden_core::Backend;
use foxden_fan::FanBackend;

pub fn lights() -> Vec<Box<dyn Backend>> {
    hp_omen::detect()
}

/// Read-only fan speeds.
pub fn fans() -> Vec<Box<dyn FanBackend>> {
    hp_omen_fans::detect()
}
