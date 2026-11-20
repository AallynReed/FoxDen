//! Bluetooth plumbing shared by the earbud, phone and fallback drivers.

pub mod adverts;
pub mod nodes;
pub mod rfcomm;

/// "L 80%, R 75%, case 40%" from optional levels; `-` for a part that did not report.
pub fn split_detail(parts: &[(&str, Option<u8>, bool)]) -> String {
    parts
        .iter()
        .map(|(label, pct, chg)| {
            let v = pct.map_or("-".to_string(), |p| format!("{p}%"));
            format!("{label} {v}{}", if *chg { " \u{26A1}" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The level to show for two earbuds: the lower one still in use, else the lower charging one.
pub fn bud_level(buds: &[(Option<u8>, bool)]) -> Option<(u8, bool)> {
    let low = |charging: bool| {
        buds.iter()
            .filter(|b| b.1 == charging)
            .filter_map(|b| b.0)
            .min()
    };
    low(false)
        .map(|p| (p, false))
        .or_else(|| low(true).map(|p| (p, true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discharging_bud_wins_over_charging_one() {
        assert_eq!(
            bud_level(&[(Some(30), true), (Some(80), false)]),
            Some((80, false))
        );
        assert_eq!(
            bud_level(&[(Some(30), true), (Some(20), true)]),
            Some((20, true))
        );
        assert_eq!(bud_level(&[(None, false), (None, true)]), None);
        assert_eq!(
            split_detail(&[("L", Some(5), true), ("R", None, false)]),
            "L 5% \u{26A1}, R -"
        );
    }
}
