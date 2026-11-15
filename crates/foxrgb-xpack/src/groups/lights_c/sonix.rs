//! SONIX keyboards: AULA, AJAZZ, Dark Project, Attack Shark, Kisnt (protocol learned from SignalRGB's SONIX_Keyboard_Controller_2.js).

mod layouts;

use super::common::open_with;
use super::common::positioned;
use super::rk::Layout;
use crate::hid::{Io, Proto, on_p};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "sonix-kb2";
const VID: u16 = 0x0C45;
const PAGE: u16 = 0xFF13;
const LEN: usize = 65;
const CHUNK: usize = 64;
const PRE: [u8; 10] = [0x00, 0x04, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08];
const APPLY: [u8; 3] = [0x00, 0x04, 0x02];

struct Model {
    product: &'static str,
    name: &'static str,
    layout: &'static Layout,
    interface: i32,
}

const fn m(
    product: &'static str,
    name: &'static str,
    layout: &'static Layout,
    interface: i32,
) -> Model {
    Model {
        product,
        name,
        layout,
        interface,
    }
}

/// Models by the HID product string the keyboard reports.
static MODELS: &[Model] = &[
    m("AULA F75Max", "AULA F75 Max", &layouts::F75_MAX, 3),
    m("AK820", "AK820", &layouts::AK820, 3),
    m("AK820MAX", "AK820 Max", &layouts::AK820, 3),
    m("AK820 MAX", "AK820 Max", &layouts::AK820, 3),
    m("AK980 PRO", "AK980 PRO", &layouts::AK980, 3),
    m("AJAZZ AK980 PRO 2.4G", "AK980 PRO", &layouts::AK980, 3),
    m(
        "AJAZZ AK35I V3 MAX",
        "AJAZZ AK35I V3 MAX",
        &layouts::FULL,
        3,
    ),
    m("AULA F108Pro", "AULA F108 Pro", &layouts::FULL, 3),
    m(
        "DP-KD-87A-000102-GMS",
        "Dark Project KD87A",
        &layouts::TKL,
        3,
    ),
    m("AULA L99", "AULA L99", &layouts::AULA_L99, 3),
    m("AULA-F98Pro V3", "AULA F98 PRO V3", &layouts::AULA_F98, 3),
    m("AULA-S75Pro", "AULA S75 Pro", &layouts::S75_PRO, 3),
    m("Attack Shark X66", "Attack Shark X66", &layouts::X66, 2),
    m("KN85 Keyboard", "Kisnt KN85", &layouts::KN85, 2),
];

/// Key data: four bytes per wire slot (slot, R, G, B), up to the highest slot used.
pub fn key_data(layout: &Layout, colors: &[Color]) -> Vec<u8> {
    let top = layout.slots.iter().copied().max().unwrap_or(0) as usize;
    let mut d = vec![0u8; (top + 1) * 4];
    for (&slot, c) in layout.slots.iter().zip(colors) {
        let at = slot as usize * 4;
        d[at..at + 4].copy_from_slice(&[slot as u8, c.0, c.1, c.2]);
    }
    d
}

struct Sonix(&'static Layout);

impl Sonix {
    fn send(&self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&PRE, LEN)?;
        io.get_feature(0x00, LEN)?;
        for chunk in key_data(self.0, colors).chunks(CHUNK) {
            let mut p = vec![0x00];
            p.extend_from_slice(chunk);
            io.feature(&p, LEN)?;
            io.sleep(2);
        }
        io.feature(&[0x00], LEN)?;
        io.feature(&APPLY, LEN)?;
        io.get_feature(0x00, LEN)?;
        Ok(())
    }
}

impl Proto for Sonix {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.send(io, colors)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.send(io, &[]);
    }
}

fn open(api: &HidApi, info: &Info, _: &Detector) -> Result<Box<dyn Backend>, String> {
    let model = MODELS
        .iter()
        .find(|m| m.product == info.product && m.interface == info.interface)
        .ok_or_else(|| format!("unknown SONIX keyboard \"{}\"", info.product))?;
    let l = model.layout;
    let dev = positioned(
        &info.device_id(FAMILY),
        model.name,
        DeviceKind::Keyboard,
        l.w,
        l.h,
        l.pos,
    );
    open_with(api, info, FAMILY, dev, Sonix(l))
}

const fn d(pid: u16) -> Detector {
    on_p("SONIX Keyboard", VID, pid, PAGE, Some(1), open)
}

pub static DETECTORS: &[Detector] = &[d(0x8006), d(0x8009), d(0x800A), d(0x8801), d(0x80B1)];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::super::rk::check;
    use super::*;

    #[test]
    fn layouts_fit() {
        for m in MODELS {
            check(m.layout, 128 * 4, 4);
        }
        assert_eq!(layouts::TKL.slots.len(), 90);
    }

    #[test]
    fn frame_sequence() {
        let l = &layouts::X66;
        let mut c = vec![Color(0, 0, 0); l.slots.len()];
        c[0] = Color(10, 20, 30);
        let mut io = Rec::default();
        Sonix(l).frame(&mut io, &c).unwrap();
        let n: usize = (121 + 1) * 4;
        let chunks = n.div_ceil(64);
        assert_eq!(io.ops.len(), 2 + chunks + 3);
        assert_eq!(io.ops[0], Op::Feature(crate::hid::padded(&PRE, 65)));
        assert_eq!(io.ops[1], Op::GetFeature(0, 65));
        let Op::Feature(first) = &io.ops[2] else {
            panic!()
        };
        assert_eq!(first[1 + 4..1 + 8], [1, 10, 20, 30]);
        assert_eq!(io.ops[2 + chunks], Op::Feature(vec![0; 65]));
        assert_eq!(
            io.ops[3 + chunks],
            Op::Feature(crate::hid::padded(&APPLY, 65))
        );
        assert_eq!(io.ops[4 + chunks], Op::GetFeature(0, 65));
    }
}
