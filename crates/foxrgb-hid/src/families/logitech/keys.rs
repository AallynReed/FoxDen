//! Ported from OpenRGB Controllers/LogitechController/LogitechG915Controller/RGBController_LogitechG915.cpp (GPL-2.0-or-later).

use std::collections::BTreeMap;

use foxrgb_core::Color;

/// Empty matrix cell.
pub const NA: u8 = 0xFF;

pub const KEYBOARD: u8 = 0x01;
pub const MEDIA: u8 = 0x02;
pub const GKEYS: u8 = 0x03;
pub const MODIFIERS: u8 = 0x04;
pub const LOGO: u8 = 0x10;
pub const INDICATORS: u8 = 0x40;

pub const MODIFIER_KEYS: [(u8, u8); 8] = [
    (MODIFIERS, 0xE0),
    (MODIFIERS, 0xE1),
    (MODIFIERS, 0xE2),
    (MODIFIERS, 0xE3),
    (MODIFIERS, 0xE4),
    (MODIFIERS, 0xE5),
    (MODIFIERS, 0xE6),
    (MODIFIERS, 0xE7),
];
pub const MEDIA_KEYS: [(u8, u8); 4] = [(MEDIA, 0x9E), (MEDIA, 0x9B), (MEDIA, 0x9D), (MEDIA, 0x9C)];
pub const G_KEYS: [(u8, u8); 5] = [(GKEYS, 1), (GKEYS, 2), (GKEYS, 3), (GKEYS, 4), (GKEYS, 5)];

/// HID usages `from..=to` on the main key zone.
pub fn usages(from: u8, to: u8) -> impl Iterator<Item = (u8, u8)> {
    (from..=to).map(|u| (KEYBOARD, u))
}

/// Key code a direct frame carries for an LED, from DeviceUpdateLEDs.
pub fn code(zone: u8, idx: u8) -> u8 {
    match zone {
        GKEYS => idx.wrapping_add(0xB3),
        MODIFIERS => idx.wrapping_sub(0x78),
        KEYBOARD => idx.wrapping_sub(0x03),
        LOGO => idx.wrapping_add(0xD1),
        _ => idx,
    }
}

/// Changed keys grouped by colour, ordered like OpenRGB's `std::map<RGBColor, ...>`.
/// With no previous frame every key counts as changed.
pub fn by_color(leds: &[(u8, u8)], prev: Option<&[Color]>, new: &[Color]) -> Vec<(Color, Vec<u8>)> {
    let mut groups: BTreeMap<u32, (Color, Vec<u8>)> = BTreeMap::new();
    for (i, &(zone, idx)) in leds.iter().enumerate() {
        let c = super::at(new, i);
        if prev.is_some_and(|p| super::at(p, i) == c) {
            continue;
        }
        let key = c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16;
        groups
            .entry(key)
            .or_insert((c, Vec::new()))
            .1
            .push(code(zone, idx));
    }
    groups.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The index switch in RGBController_LogitechG915::DeviceUpdateLEDs (same in G815).
    #[test]
    fn key_codes_match_openrgb() {
        assert_eq!(code(KEYBOARD, 0x04), 0x01);
        assert_eq!(code(KEYBOARD, 0x29), 0x26);
        assert_eq!(code(MODIFIERS, 0xE0), 0x68);
        assert_eq!(code(GKEYS, 0x01), 0xB4);
        assert_eq!(code(LOGO, 0x01), 0xD2);
        assert_eq!(code(MEDIA, 0x9E), 0x9E);
        assert_eq!(code(INDICATORS, 0x99), 0x99);
    }

    // RGBColor is R | G << 8 | B << 16, so std::map orders by blue, then green, then red.
    #[test]
    fn groups_follow_rgbcolor_order() {
        let leds = [(KEYBOARD, 4), (KEYBOARD, 5), (KEYBOARD, 6), (KEYBOARD, 7)];
        let colors = [
            Color(0, 0, 1),
            Color(9, 0, 0),
            Color(0, 0, 1),
            Color(0, 1, 0),
        ];
        let g = by_color(&leds, None, &colors);
        assert_eq!(
            g,
            vec![
                (Color(9, 0, 0), vec![2]),
                (Color(0, 1, 0), vec![4]),
                (Color(0, 0, 1), vec![1, 3]),
            ]
        );
        let again = by_color(&leds, Some(&colors), &colors);
        assert!(again.is_empty());
    }
}
