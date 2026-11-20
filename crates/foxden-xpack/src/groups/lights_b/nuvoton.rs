//! Nuvoton keyboards (Drevo Tyrfing v2) (protocol learned from SignalRGB's
//! Nuvoton_Keyboard2_Controller.js).

use super::layouts::{NUVO_FULL_MAP, NUVO_FULL_POS, NUVO_TKL_MAP, NUVO_TKL_POS};
use super::{KEYBOARD, det, placed};
use crate::hid::{Driver, Io, Proto, drain, open_io, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "nuvoton";
const REPORT: u8 = 0x06;
const LEN: usize = 32;
const PER_REPORT: usize = 5;

pub static DETECTORS: &[Detector] = &[det(
    "Nuvoton Keyboard",
    0x0416,
    0xA0F8,
    Some(1),
    Some(0x0001),
    Some(0x0000),
    open,
)];

struct Layout {
    size: [u32; 2],
    pos: &'static [[u8; 2]],
    map: &'static [u8],
}

const FULL: Layout = Layout {
    size: [21, 6],
    pos: NUVO_FULL_POS,
    map: NUVO_FULL_MAP,
};
const TKL: Layout = Layout {
    size: [17, 6],
    pos: NUVO_TKL_POS,
    map: NUVO_TKL_MAP,
};

/// Model strings the firmware reports, with the product and its layout.
const MODELS: &[(&str, &str, &Layout)] = &[
    ("WKB-NUC-NP87B-US", "Drevo Tyrfing v2 TKL", &TKL),
    ("WKNUB-NP87-UK", "Drevo Tyrfing v2 TKL", &TKL),
    ("WKNUB-ZK104-UK", "Drevo Tyrfing v2", &FULL),
];

/// `fetchFirmwareData`: the model string sits in bytes 4..20 of the BE 02 reply.
pub fn model_name(reply: &[u8]) -> String {
    reply
        .iter()
        .skip(4)
        .take(16)
        .filter(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

/// `sendColors`: up to five firmware key ids (zero padded), then R, G, B, 0 for each.
pub fn reports(map: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    map.chunks(PER_REPORT)
        .zip(colors.chunks(PER_REPORT))
        .map(|(ids, cs)| {
            let mut b = vec![REPORT, 0xBE, 0x19, 0x00, 0x01, 0x01, 0x0E];
            b.extend(ids);
            b.resize(7 + PER_REPORT, 0);
            for &c in cs {
                b.extend(rgb(c));
                b.push(0);
            }
            b
        })
        .collect()
}

struct Nuvoton {
    map: &'static [u8],
}

impl Proto for Nuvoton {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&[REPORT, 0xBE, 0x14, 0x00, 0x01, 0x00, 0x0E], LEN)?;
        io.read(LEN, 100)?;
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        for r in reports(self.map, c) {
            io.write(&r, LEN)?;
            io.sleep(2);
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &vec![Color(0, 0, 0); self.map.len()]);
    }
}

fn open(api: &HidApi, info: &Info, _d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    drain(io.as_mut(), LEN);
    io.write(&[REPORT, 0xBE, 0x02], LEN)?;
    let name = model_name(&io.read(LEN, 500)?);
    let &(_, product, layout) = MODELS
        .iter()
        .find(|m| m.0 == name)
        .ok_or_else(|| format!("unknown Nuvoton model {name:?}"))?;
    let dev = placed(
        info,
        FAMILY,
        product,
        DeviceKind::Keyboard,
        KEYBOARD,
        layout.size,
        layout.pos,
    );
    Ok(Driver::new(io, dev, "nuvoton", Nuvoton { map: layout.map }).boxed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_keys_per_report() {
        let r = reports(&[41, 58, 59, 60, 61, 62], &[Color(1, 2, 3); 6]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].len(), 32);
        assert_eq!(
            r[0][..16],
            [6, 0xBE, 0x19, 0, 1, 1, 0x0E, 41, 58, 59, 60, 61, 1, 2, 3, 0]
        );
        assert_eq!(r[1][7..13], [62, 0, 0, 0, 0, 1]);
        assert_eq!(NUVO_FULL_POS.len(), NUVO_FULL_MAP.len());
        assert_eq!(NUVO_TKL_POS.len(), NUVO_TKL_MAP.len());
    }

    #[test]
    fn model_from_reply() {
        let mut r = vec![6, 0xBE, 2, 0];
        r.extend(b"WKNUB-NP87-UK");
        r.resize(32, 0);
        assert_eq!(model_name(&r), "WKNUB-NP87-UK");
    }
}
