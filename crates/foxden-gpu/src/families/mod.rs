//! Graphics card families ported from OpenRGB, each in its own folder.

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

    /// Cards OpenRGB itself registers with two drivers, each probing its own address.
    const OPENRGB_TWICE: [[u16; 4]; 1] = [[0x10DE, 0x2489, 0x1458, 0x405A]];
    /// OpenRGB lists these Blackwell and RX 9000 cards under both Fusion 2 drivers.
    const OPENRGB_PAIR: [&str; 2] = ["gigabyte_fusion2", "gigabyte_blackwell"];

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
                        && !OPENRGB_TWICE.contains(&ids)
                        && !(OPENRGB_PAIR.contains(&other) && OPENRGB_PAIR.contains(&family)) =>
                {
                    clashes.push(format!("{ids:04x?} {other} / {family}"))
                }
                _ => {}
            }
        }
        assert!(clashes.is_empty(), "{clashes:#?}");
    }
}
