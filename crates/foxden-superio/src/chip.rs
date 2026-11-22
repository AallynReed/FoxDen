//! Super I/O chip identities and how each one is recognised.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Chip {
    F71808E,
    F71858,
    F71862,
    F71869,
    F71869A,
    F71878AD,
    F71882,
    F71889AD,
    F71889ED,
    F71889F,
    IT8613E,
    IT8620E,
    IT8625E,
    IT8628E,
    IT8631E,
    IT8638E,
    IT8655E,
    IT8665E,
    IT8686E,
    IT8688E,
    IT8689E,
    IT8696E,
    IT8705F,
    IT8712F,
    IT8716F,
    IT8718F,
    IT8720F,
    IT8721F,
    IT8726F,
    IT8728F,
    IT8771E,
    IT8772E,
    IT8790E,
    IT8792E,
    IT87952E,
    NCT5585D,
    NCT610XD,
    NCT6771F,
    NCT6776F,
    NCT6779D,
    NCT6791D,
    NCT6792D,
    NCT6792DA,
    NCT6793D,
    NCT6795D,
    NCT6796D,
    NCT6796DR,
    NCT6796DS,
    NCT6797D,
    NCT6798D,
    NCT6799D,
    NCT6701D,
    NCT6683D,
    NCT6686D,
    NCT6687D,
    NCT6687DR,
    W83627DHG,
    W83627DHGP,
    W83627EHF,
    W83627HF,
    W83627THF,
    W83667HG,
    W83667HGB,
    W83687THF,
}

use Chip::*;

/// Logical device of the hardware monitor.
pub(crate) const NUVOTON_LDN: u8 = 0x0B;
const FINTEK_LDN: u8 = 0x04;
const F71858_LDN: u8 = 0x02;

impl Chip {
    /// Lower-case name, used in sensor and fan ids.
    pub fn name(self) -> String {
        let name = format!("{self:?}").to_lowercase();
        if name == "nct610xd" {
            "nct610x".into()
        } else {
            name
        }
    }

    pub fn is_ite(self) -> bool {
        format!("{self:?}").starts_with("IT")
    }

    pub fn is_fintek(self) -> bool {
        format!("{self:?}").starts_with('F')
    }

    pub fn is_winbond(self) -> bool {
        format!("{self:?}").starts_with('W')
    }

    /// The Nuvoton chips run by the embedded controller, reached through pages.
    pub fn is_paged(self) -> bool {
        matches!(self, NCT6683D | NCT6686D | NCT6687D | NCT6687DR)
    }

    /// Chips with a hardware monitor I/O space lock.
    pub fn has_io_lock(self) -> bool {
        matches!(
            self,
            NCT6791D
                | NCT6792D
                | NCT6792DA
                | NCT6793D
                | NCT6795D
                | NCT6796D
                | NCT6796DR
                | NCT6796DS
                | NCT6797D
                | NCT6798D
                | NCT6799D
                | NCT6701D
                | NCT5585D
        )
    }
}

/// What the board needs to be told apart, for ids that more than one chip answers with.
#[derive(Clone, Debug, Default)]
pub(crate) struct Board {
    pub maker: crate::boards::Maker,
    pub product: String,
    pub model: Option<&'static str>,
    pub amd: bool,
}

/// A Winbond, Nuvoton or Fintek chip from its id and revision, with its monitor's logical device.
pub(crate) fn winbond_family(id: u8, rev: u8, board: &Board) -> Option<(Chip, u8)> {
    let model = board.model.unwrap_or("");
    let fintek = |chip| Some((chip, FINTEK_LDN));
    let chip = match (id, rev) {
        (0x05, 0x07) => return Some((F71858, F71858_LDN)),
        (0x05, 0x41) => return fintek(F71882),
        (0x06, 0x01) => return fintek(F71862),
        (0x07, 0x23) => return fintek(F71889F),
        (0x08, 0x14) => return fintek(F71869),
        (0x09, 0x01) => return fintek(F71808E),
        (0x09, 0x09) => return fintek(F71889ED),
        (0x10, 0x05) => return fintek(F71889AD),
        (0x10, 0x07) | (0x11, 0x18) => return fintek(F71869A),
        (0x11, 0x06) => return fintek(F71878AD),
        (0x52, 0x17 | 0x3A | 0x41) => W83627HF,
        (0x82, r) if r & 0xF0 == 0x80 => W83627THF,
        (0x85, 0x41) => W83687THF,
        (0x88, r) if matches!(r & 0xF0, 0x50 | 0x60) => W83627EHF,
        (0xA0, r) if r & 0xF0 == 0x20 => W83627DHG,
        (0xA5, r) if r & 0xF0 == 0x10 => W83667HG,
        (0xB0, r) if r & 0xF0 == 0x70 => W83627DHGP,
        (0xB3, r) if r & 0xF0 == 0x50 => W83667HGB,
        (0xB4, r) if r & 0xF0 == 0x70 => NCT6771F,
        (0xC3, r) if r & 0xF0 == 0x30 => NCT6776F,
        (0xC4, r) if r & 0xF0 == 0x50 => NCT610XD,
        (0xC5, r) if r & 0xF0 == 0x60 => NCT6779D,
        (0xC7, 0x32) => NCT6683D,
        (0xC8, 0x03) => NCT6791D,
        (0xC9, 0x11) => NCT6792D,
        (0xC9, 0x13) => NCT6792DA,
        (0xD1, 0x21) => NCT6793D,
        (0xD3, 0x52) => NCT6795D,
        (0xD4, 0x23) => NCT6796D,
        (0xD4, 0x2A) if model == "X870E_NOVA_WIFI" => NCT5585D,
        (0xD4, 0x2A) => NCT6796DR,
        (0xD4, 0x51) => NCT6797D,
        (0xD4, 0x2B) => NCT6798D,
        (0xD4, 0x40 | 0x41) => NCT6686D,
        (0xD5, 0x92) if msi_800_series(board) => NCT6687DR,
        (0xD5, 0x92) => NCT6687D,
        (0xD8, 0x02) if matches!(model, "X870E_NOVA_WIFI" | "B650M_HDV_M_2") => NCT6796DS,
        (0xD8, 0x02) => NCT6799D,
        (0xD8, 0x06) => NCT6701D,
        _ => return None,
    };
    Some((chip, NUVOTON_LDN))
}

/// MSI boards of the 800 series use a later firmware on the NCT6687D.
fn msi_800_series(board: &Board) -> bool {
    let product = board.product.to_uppercase();
    board.maker == crate::boards::Maker::MSI
        && ["B840", "B850", "B860", "X870", "Z890"]
            .iter()
            .any(|series| product.contains(series))
}

pub(crate) fn ite(id: u16) -> Option<Chip> {
    Some(match id {
        0x8613 => IT8613E,
        0x8620 => IT8620E,
        0x8625 => IT8625E,
        0x8628 => IT8628E,
        0x8631 => IT8631E,
        0x8638 => IT8638E,
        0x8655 => IT8655E,
        0x8665 => IT8665E,
        0x8686 => IT8686E,
        0x8688 => IT8688E,
        0x8689 => IT8689E,
        0x8696 => IT8696E,
        0x8705 => IT8705F,
        0x8712 => IT8712F,
        0x8716 => IT8716F,
        0x8718 => IT8718F,
        0x8720 => IT8720F,
        0x8721 => IT8721F,
        0x8726 => IT8726F,
        0x8728 | 0x8784 => IT8728F,
        0x8771 => IT8771E,
        0x8772 => IT8772E,
        0x8790 => IT8790E,
        0x8733 => IT8792E,
        0x8695 => IT87952E,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boards::Maker;

    #[test]
    fn every_id_and_revision() {
        let plain = Board::default();
        let fintek = [
            (0x05, 0x07, F71858),
            (0x05, 0x41, F71882),
            (0x06, 0x01, F71862),
            (0x07, 0x23, F71889F),
            (0x08, 0x14, F71869),
            (0x09, 0x01, F71808E),
            (0x09, 0x09, F71889ED),
            (0x10, 0x05, F71889AD),
            (0x10, 0x07, F71869A),
            (0x11, 0x18, F71869A),
            (0x11, 0x06, F71878AD),
        ];
        for (id, rev, chip) in fintek {
            let ldn = if chip == F71858 {
                F71858_LDN
            } else {
                FINTEK_LDN
            };
            assert_eq!(
                winbond_family(id, rev, &plain),
                Some((chip, ldn)),
                "{id:x} {rev:x}"
            );
        }
        let nuvoton = [
            (0x52, 0x17, W83627HF),
            (0x52, 0x3A, W83627HF),
            (0x52, 0x41, W83627HF),
            (0x82, 0x83, W83627THF),
            (0x85, 0x41, W83687THF),
            (0x88, 0x51, W83627EHF),
            (0x88, 0x61, W83627EHF),
            (0xA0, 0x21, W83627DHG),
            (0xA5, 0x12, W83667HG),
            (0xB0, 0x73, W83627DHGP),
            (0xB3, 0x51, W83667HGB),
            (0xB4, 0x70, NCT6771F),
            (0xC3, 0x33, NCT6776F),
            (0xC4, 0x51, NCT610XD),
            (0xC5, 0x62, NCT6779D),
            (0xC7, 0x32, NCT6683D),
            (0xC8, 0x03, NCT6791D),
            (0xC9, 0x11, NCT6792D),
            (0xC9, 0x13, NCT6792DA),
            (0xD1, 0x21, NCT6793D),
            (0xD3, 0x52, NCT6795D),
            (0xD4, 0x23, NCT6796D),
            (0xD4, 0x2A, NCT6796DR),
            (0xD4, 0x51, NCT6797D),
            (0xD4, 0x2B, NCT6798D),
            (0xD4, 0x40, NCT6686D),
            (0xD4, 0x41, NCT6686D),
            (0xD5, 0x92, NCT6687D),
            (0xD8, 0x02, NCT6799D),
            (0xD8, 0x06, NCT6701D),
        ];
        for (id, rev, chip) in nuvoton {
            assert_eq!(
                winbond_family(id, rev, &plain),
                Some((chip, NUVOTON_LDN)),
                "{id:x} {rev:x}"
            );
        }
        for (id, rev) in [
            (0x00, 0x00),
            (0xFF, 0xFF),
            (0xC7, 0x33),
            (0x52, 0x18),
            (0xD5, 0x93),
        ] {
            assert_eq!(winbond_family(id, rev, &plain), None);
        }
        let hdv = Board {
            model: Some("B650M_HDV_M_2"),
            ..Board::default()
        };
        assert_eq!(winbond_family(0xD8, 0x02, &hdv).unwrap().0, NCT6796DS);
        let msi = |product: &str| Board {
            maker: Maker::MSI,
            product: product.into(),
            ..Board::default()
        };
        for product in [
            "PRO B840-P",
            "B850 GAMING",
            "B860M",
            "MEG X870E GODLIKE",
            "MPG Z890 EDGE",
        ] {
            assert_eq!(
                winbond_family(0xD5, 0x92, &msi(product)).unwrap().0,
                NCT6687DR
            );
        }
        assert_eq!(
            winbond_family(0xD5, 0x92, &msi("MAG B650 TOMAHAWK"))
                .unwrap()
                .0,
            NCT6687D
        );

        let ite_ids = [
            (0x8613, IT8613E),
            (0x8620, IT8620E),
            (0x8625, IT8625E),
            (0x8628, IT8628E),
            (0x8631, IT8631E),
            (0x8638, IT8638E),
            (0x8655, IT8655E),
            (0x8665, IT8665E),
            (0x8686, IT8686E),
            (0x8688, IT8688E),
            (0x8689, IT8689E),
            (0x8696, IT8696E),
            (0x8705, IT8705F),
            (0x8712, IT8712F),
            (0x8716, IT8716F),
            (0x8718, IT8718F),
            (0x8720, IT8720F),
            (0x8721, IT8721F),
            (0x8726, IT8726F),
            (0x8728, IT8728F),
            (0x8784, IT8728F),
            (0x8771, IT8771E),
            (0x8772, IT8772E),
            (0x8790, IT8790E),
            (0x8733, IT8792E),
            (0x8695, IT87952E),
        ];
        for (id, chip) in ite_ids {
            assert_eq!(ite(id), Some(chip));
            assert!(chip.is_ite());
        }
        assert_eq!(ite(0x8792), None);
        assert_eq!(ite(0xFFFF), None);
    }
}
