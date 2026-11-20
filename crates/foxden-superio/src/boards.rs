//! Who made the board and which model it is, and what that board calls its headers.

use crate::board_names::{LAYOUTS, Names, PRODUCTS, RULES};
use crate::chip::Chip;
use crate::monitor::Layout;

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Maker {
    Acer,
    AMD,
    Alienware,
    AOpen,
    Apple,
    ASRock,
    ASUS,
    Biostar,
    Clevo,
    Dell,
    DFI,
    ECS,
    EPoX,
    EVGA,
    FIC,
    Foxconn,
    Framework,
    Fujitsu,
    Gigabyte,
    HP,
    IBM,
    Intel,
    Jetway,
    Lenovo,
    LattePanda,
    Medion,
    Microsoft,
    MSI,
    NEC,
    Pegatron,
    Samsung,
    Sapphire,
    Shuttle,
    Sony,
    Supermicro,
    Toshiba,
    XFX,
    Zotac,
    #[default]
    Unknown,
}

/// The maker from the DMI baseboard manufacturer.
pub(crate) fn maker(name: &str) -> Maker {
    use Maker::*;
    let lower = name.to_lowercase();
    let starts = |prefix: &str| lower.starts_with(&prefix.to_lowercase());
    let is = |whole: &str| lower == whole.to_lowercase();
    if lower.contains("abit.com.tw") || starts("Acer") {
        return Acer;
    }
    let prefixes: [(&str, Maker); 27] = [
        ("AMD", AMD),
        ("AOpen", AOpen),
        ("Apple", Apple),
        ("ASUSTeK", ASUS),
        ("ASUS ", ASUS),
        ("Biostar", Biostar),
        ("Clevo", Clevo),
        ("Dell", Dell),
        ("DFI Inc", DFI),
        ("ELITEGROUP", ECS),
        ("EVGA", EVGA),
        ("First International Computer", FIC),
        ("Framework", Framework),
        ("Fujitsu", Fujitsu),
        ("Gigabyte", Gigabyte),
        ("Hewlett-Packard", HP),
        ("Intel Corp", Intel),
        ("Jetway", Jetway),
        ("Lenovo", Lenovo),
        ("Medion", Medion),
        ("Microsoft", Microsoft),
        ("Micro-Star International", MSI),
        ("NEC ", NEC),
        ("Pegatron", Pegatron),
        ("Samsung", Samsung),
        ("Sapphire", Sapphire),
        ("Shuttle", Shuttle),
    ];
    let more: [(&str, Maker); 4] = [
        ("Sony", Sony),
        ("Supermicro", Supermicro),
        ("Toshiba", Toshiba),
        ("Zotac", Zotac),
    ];
    let wholes: [(&str, Maker); 13] = [
        ("Alienware", Alienware),
        ("ASRock", ASRock),
        ("DFI", DFI),
        ("ECS", ECS),
        ("EPoX COMPUTER CO., LTD", EPoX),
        ("FIC", FIC),
        ("Foxconn", Foxconn),
        ("HP", HP),
        ("IBM", IBM),
        ("Intel", Intel),
        ("LattePanda", LattePanda),
        ("MSI", MSI),
        ("NEC", NEC),
    ];
    if let Some((_, m)) = wholes.iter().find(|(w, _)| is(w)) {
        return *m;
    }
    if is("XFX") {
        return XFX;
    }
    prefixes
        .iter()
        .chain(&more)
        .find(|(p, _)| starts(p))
        .map_or(Unknown, |(_, m)| *m)
}

/// The model from the DMI baseboard product name, for boards with names of their own.
pub(crate) fn model(product: &str) -> Option<&'static str> {
    let lower = product.to_lowercase();
    PRODUCTS
        .iter()
        .find(|(name, _, prefix)| {
            let name = name.to_lowercase();
            if *prefix {
                lower.starts_with(&name)
            } else {
                lower == name
            }
        })
        .map(|(_, model, _)| *model)
}

fn owned(names: Names) -> Vec<(String, usize)> {
    names
        .iter()
        .map(|&(n, i)| (n.to_string(), i as usize))
        .collect()
}

/// What this board calls the inputs of its `index`th chip of this kind.
pub(crate) fn layout(
    chip: Chip,
    maker: Maker,
    model: Option<&str>,
    index: usize,
) -> Option<Layout> {
    let rule = RULES.iter().find(|rule| {
        rule.chips.contains(&chip)
            && rule.maker.is_none_or(|m| m == maker)
            && rule.model.is_none_or(|m| Some(m) == model)
            && rule.index.is_none_or(|i| i as usize == index)
    })?;
    let (temps, fans, controls) = LAYOUTS[rule.layout as usize];
    Some(Layout {
        temps: owned(temps),
        fans: owned(fans),
        controls: owned(controls),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::Chip::*;

    #[test]
    fn makers_and_models() {
        assert_eq!(maker("Gigabyte Technology Co., Ltd."), Maker::Gigabyte);
        assert_eq!(maker("ASUSTeK COMPUTER INC."), Maker::ASUS);
        assert_eq!(maker("Micro-Star International Co., Ltd."), Maker::MSI);
        assert_eq!(maker("asrock"), Maker::ASRock);
        assert_eq!(maker("To be filled by O.E.M."), Maker::Unknown);
        assert_eq!(model("X570 AORUS MASTER"), Some("X570_AORUS_MASTER"));
        assert_eq!(model("b650m-hdv/m.2 something"), Some("B650M_HDV_M_2"));
        assert_eq!(model("Nothing like it"), None);
    }

    #[test]
    fn named_headers_for_known_boards() {
        let l = layout(IT8688E, Maker::Gigabyte, Some("X570_AORUS_MASTER"), 0).unwrap();
        assert_eq!(l.fans[0], ("CPU fan".to_string(), 0));
        assert!(l.controls.iter().any(|c| c.0 == "CPU optional fan"));
        // the second chip on a dual-chip board gets its own names
        let second = layout(IT8792E, Maker::Gigabyte, Some("B550_AORUS_MASTER"), 0).unwrap();
        assert_eq!(second.fans[0].0, "System fan 5 / pump");
        // a board without names of its own falls back to the chip's
        let plain = layout(IT8688E, Maker::Unknown, None, 0).unwrap();
        assert!(!plain.fans.is_empty());
        // headers a board only reads are not given a control
        let dr = layout(NCT6687DR, Maker::MSI, Some("X870E_TOMAHAWK_WIFI"), 0).unwrap();
        assert!(dr.fans.iter().any(|f| f.1 == 10));
        assert!(dr.controls.iter().all(|c| ![4, 5, 6, 7, 8].contains(&c.1)));
    }
}
