//! Ported from OpenRGB Controllers/*GPUController (GPL-2.0-or-later).
//!
//! Not ported: ColorfulGPUController, ZotacV2GPUController and ManliGPUController detect with
//! a raw I2C read, and ZotacTuringGPUController reads back raw after every write; the SmBus
//! trait has no raw read.

use foxrgb_smbus::i2c::Detect;

#[rustfmt::skip]
mod cards;
mod colorful_turing;
mod common;
mod evga_ampere;
mod evga_gp102;
mod evga_pascal;
mod evga_turing;
#[cfg(test)]
mod fake;
mod galax;
mod palit;
mod pny_epicx;
mod pny_lovelace;
mod powercolor;
mod sapphire;
mod zotac_blackwell;

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
    ]
}

#[cfg(test)]
mod tests {
    use foxrgb_core::Color;

    use super::cards::*;
    use super::fake::{Fake, ids, push, run, take};

    // DetectionManager::RegisterI2CPCIDeviceDetector: one row per line in each *Detect.cpp.
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
        ];
        assert_eq!(sizes, [68, 60, 26, 8, 5, 2, 33, 3, 12, 7, 11, 10]);
    }

    // EVGAGPUv1Controller::SetMode and SetColor: a failed write is retried on the next push.
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
