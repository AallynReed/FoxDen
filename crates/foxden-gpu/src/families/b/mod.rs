// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_smbus::i2c::Detect;

mod asrock;
#[rustfmt::skip]
mod cards;
mod colorful;
mod colorful_turing;
mod common;
mod evga_ampere;
mod evga_gp102;
mod evga_pascal;
mod evga_turing;
#[cfg(test)]
mod fake;
mod galax;
mod gigabyte_blackwell;
mod gigabyte_fusion2;
mod gigabyte_xtreme;
mod lenovo_legion;
mod manli;
mod msi;
mod msi_trio;
mod palit;
mod pny_epicx;
mod pny_lovelace;
mod pny_revel;
mod powercolor;
mod sapphire;
mod zotac_blackwell;
mod zotac_turing;
mod zotac_v2;

/// Whether any table here has a card with these PCI ids.
pub fn known(ids: [u16; 4]) -> bool {
    cards::ALL
        .iter()
        .flat_map(|t| t.iter())
        .any(|c| [c.ven, c.dev, c.subven, c.subdev] == ids)
        || palit::unlisted_known(ids)
}

pub fn all() -> Vec<Detect> {
    vec![
        palit::detect_v1,
        palit::detect_v2,
        evga_ampere::detect,
        evga_turing::detect,
        evga_gp102::detect,
        evga_pascal::detect,
        colorful_turing::detect,
        sapphire::detect_v1,
        sapphire::detect_v3,
        zotac_blackwell::detect,
        pny_lovelace::detect,
        pny_epicx::detect_small,
        pny_epicx::detect_large,
        powercolor::detect_v1,
        powercolor::detect_v2,
        galax::detect,
        colorful::detect,
        manli::detect,
        zotac_turing::detect,
        zotac_v2::detect,
        msi::detect_v1,
        msi::detect_v2,
        gigabyte_fusion2::detect,
        gigabyte_blackwell::detect_single,
        gigabyte_blackwell::detect_gaming,
        gigabyte_blackwell::detect_waterforce,
        gigabyte_blackwell::detect_aorus_waterforce,
        gigabyte_blackwell::detect_master_5080,
        gigabyte_blackwell::detect_master_5090d_ice,
        gigabyte_xtreme::detect,
        lenovo_legion::detect,
        msi_trio::detect,
        pny_revel::detect,
        asrock::detect,
    ]
}

/// Every card listed, with the driver file it belongs to.
#[cfg(test)]
pub fn cards() -> Vec<(&'static str, [u16; 4])> {
    use cards::*;
    let families: [(&str, &[&[common::Card]]); 24] = [
        ("palit", &[PALIT_V1, PALIT_V2]),
        ("evga_ampere", &[EVGA_AMPERE]),
        ("evga_turing", &[EVGA_TURING]),
        ("evga_gp102", &[EVGA_GP102]),
        ("evga_pascal", &[EVGA_PASCAL]),
        ("sapphire", &[SAPPHIRE_V1, SAPPHIRE_V3]),
        ("zotac_blackwell", &[ZOTAC_BLACKWELL]),
        ("pny_lovelace", &[PNY_LOVELACE]),
        ("pny_epicx", &[PNY_EPICX_SMALL, PNY_EPICX_LARGE]),
        ("powercolor", &[POWERCOLOR_V1, POWERCOLOR_V2]),
        ("galax", &[GALAX]),
        ("colorful_turing", &[COLORFUL_TURING]),
        ("colorful", &[COLORFUL]),
        ("manli", &[MANLI]),
        ("zotac_turing", &[ZOTAC_TURING]),
        ("zotac_v2", &[ZOTAC_V2]),
        ("msi", &[MSI_V1, MSI_V2]),
        ("gigabyte_fusion2", &[FUSION2]),
        (
            "gigabyte_blackwell",
            &[
                FUSION2_BLACKWELL_SINGLE,
                FUSION2_BLACKWELL_GAMING,
                FUSION2_BLACKWELL_MASTER_5080,
                FUSION2_BLACKWELL_AORUS_WATERFORCE,
                FUSION2_BLACKWELL_WATERFORCE,
                FUSION2_BLACKWELL_MASTER_5090D_ICE,
            ],
        ),
        ("gigabyte_xtreme", &[gigabyte_xtreme::GIGABYTE_XTREME]),
        ("lenovo_legion", &[lenovo_legion::LENOVO_LEGION]),
        ("msi_trio", &[msi_trio::MSI_BLACKWELL_TRIO]),
        ("pny_revel", &[pny_revel::PNY_REVEL]),
        ("asrock", &[asrock::ASROCK]),
    ];
    let listed: usize = families.iter().flat_map(|f| f.1).map(|t| t.len()).sum();
    assert_eq!(listed, ALL.iter().map(|t| t.len()).sum::<usize>());
    families
        .iter()
        .flat_map(|&(name, tables)| {
            tables
                .iter()
                .flat_map(|t| t.iter())
                .map(move |c| (name, [c.ven, c.dev, c.subven, c.subdev]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use foxden_core::Color;

    use super::cards::*;
    use super::fake::{Fake, ids, push, run, take};

    // One row per detector.
    #[test]
    fn table_sizes() {
        let sizes = [
            PALIT_V1.len() + PALIT_V2.len(),
            EVGA_AMPERE.len(),
            EVGA_TURING.len(),
            EVGA_GP102.len(),
            EVGA_PASCAL.len(),
            COLORFUL_TURING.len(),
            SAPPHIRE_V1.len() + SAPPHIRE_V3.len(),
            ZOTAC_BLACKWELL.len(),
            PNY_LOVELACE.len(),
            PNY_EPICX_SMALL.len() + PNY_EPICX_LARGE.len(),
            POWERCOLOR_V1.len() + POWERCOLOR_V2.len(),
            GALAX.len(),
            COLORFUL.len(),
            MANLI.len(),
            ZOTAC_TURING.len(),
            ZOTAC_V2.len(),
            FUSION2.len(),
            [
                FUSION2_BLACKWELL_SINGLE,
                FUSION2_BLACKWELL_GAMING,
                FUSION2_BLACKWELL_WATERFORCE,
                FUSION2_BLACKWELL_AORUS_WATERFORCE,
                FUSION2_BLACKWELL_MASTER_5080,
                FUSION2_BLACKWELL_MASTER_5090D_ICE,
            ]
            .iter()
            .map(|t| t.len())
            .sum(),
            MSI_V1.len(),
            MSI_V2.len(),
            super::gigabyte_xtreme::GIGABYTE_XTREME.len(),
            super::lenovo_legion::LENOVO_LEGION.len(),
            super::msi_trio::MSI_BLACKWELL_TRIO.len(),
            super::pny_revel::PNY_REVEL.len(),
            super::asrock::ASROCK.len(),
        ];
        // Base counts, then additions; MSI_V2 hands seven GAMING TRIO cards to msi_trio.
        assert_eq!(
            sizes,
            [
                68 + 15,
                60 + 9,
                26 + 12,
                8,
                5,
                2,
                33,
                3,
                12 + 6,
                7 + 1,
                11,
                10,
                40 + 23,
                1,
                4,
                17,
                103,
                32,
                83 + 16,
                47 - 7 + 13,
                8,
                2,
                10,
                1,
                2
            ]
        );
        // known() reads cards::ALL, so it must hold every table above
        let all: usize = ALL.iter().map(|t| t.len()).sum();
        assert_eq!(all, sizes.iter().sum::<usize>());
    }

    // v1 mode and colour: a failed write is retried on the next push.
    #[test]
    fn failed_write_retries_setup() {
        let mut fake = Fake::new("NV", ids(EVGA_PASCAL, "EVGA GeForce GTX 1080 FTW"));
        fake.fail_writes = true;
        let mut b = run(&super::all(), &fake.shared()).remove(0);
        let id = b.devices()[0].id.clone();
        assert!(b.push(&id, &[Color(1, 2, 3)]).is_err());
        assert_eq!(take().len(), 4);
        assert!(b.push(&id, &[Color(1, 2, 3)]).is_err());
        assert_eq!(take().len(), 4);
        assert!(b.push(&id, &[Color(1, 2, 3), Color::BLACK]).is_err());
        let ok = Fake::new("NV", ids(EVGA_PASCAL, "EVGA GeForce GTX 1080 FTW")).shared();
        let mut b = run(&super::all(), &ok).remove(0);
        assert_eq!(push(&mut b, &[Color(1, 2, 3)]).len(), 4);
    }
}
