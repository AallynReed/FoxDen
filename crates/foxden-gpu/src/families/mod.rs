// SPDX-License-Identifier: GPL-2.0-or-later
//! Graphics card families, each in its own folder.

use foxden_smbus::i2c::Detect;

mod a;
mod b;

pub fn known(ids: [u16; 4]) -> bool {
    a::known(ids) || b::known(ids) || foxden_smbus::gpu_cards().contains(&ids)
}

pub fn all() -> Vec<Detect> {
    [a::all(), b::all(), foxden_smbus::gpu_families()].concat()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    /// Cards registered with two drivers, each probing its own address.
    const TWO_DRIVERS: [[u16; 4]; 1] = [[0x10DE, 0x2489, 0x1458, 0x405A]];
    /// Blackwell and RX 9000 cards listed under both Fusion 2 drivers.
    const FUSION2_PAIR: [&str; 2] = ["gigabyte_fusion2", "gigabyte_blackwell"];

    // A card may sit at several addresses within one driver, but only one driver lists it.
    #[test]
    fn no_card_in_two_families() {
        let mut all: Vec<_> = foxden_smbus::gpu_cards()
            .into_iter()
            .map(|ids| ("ene", ids))
            .collect();
        all.extend(super::a::cards());
        all.extend(super::b::cards());
        let mut owner: HashMap<[u16; 4], &str> = HashMap::new();
        let mut clashes = Vec::new();
        for (family, ids) in all {
            match owner.insert(ids, family) {
                Some(other)
                    if other != family
                        && !TWO_DRIVERS.contains(&ids)
                        && !(FUSION2_PAIR.contains(&other) && FUSION2_PAIR.contains(&family)) =>
                {
                    clashes.push(format!("{ids:04x?} {other} / {family}"))
                }
                _ => {}
            }
        }
        assert!(clashes.is_empty(), "{clashes:#?}");
    }
}
