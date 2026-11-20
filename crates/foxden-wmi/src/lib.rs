//! Ported from OpenRGB Controllers/HPOmenLaptopController (GPL-2.0-or-later).
//! Laptop lighting through vendor WMI methods. Needs administrator rights, or the installed service.

mod hp_omen;

use foxden_core::Backend;

pub fn lights() -> Vec<Box<dyn Backend>> {
    hp_omen::detect()
}
