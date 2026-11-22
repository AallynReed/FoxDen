//! Sinowealth keyboards, Redragon and others.

mod layouts;

use super::common::positioned;
use super::rk::{Collections, Layout, vendor};
use crate::hid::{Driver, Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "sinowealth-kb2";
const QUERY: [u8; 6] = [0x05, 0x05, 0x81, 0x00, 0x00, 0x00];
const QUERY_REPLY: usize = 8;
const COLORS_HEAD: [u8; 4] = [0x08, 0x0A, 0x7A, 0x01];
const COLORS_LEN: usize = 382;

/// Firmware model id → (name, layout).
fn model(id: u32) -> Option<(&'static str, &'static Layout)> {
    use layouts::*;
    Some(match id {
        13 => ("Redragon K633 Ryze Pro", &L68),
        19 => ("Redragon K633 Ryze", &L68),
        341 => ("Redragon K565 Rudra", &FULL),
        354 => ("Redragon K641 Shaco Pro", &L68_V2),
        131 => ("Redragon K653 Sion Pro", &L96),
        467 => ("Redragon K668 Trundle", &FULLV2),
        13059 => ("Redragon K618 Horus", &FULL),
        12 => ("Redragon K619 Horus", &FULL),
        371 => ("AULA F3261", &L60),
        1 => ("CYI GAS67", &L68),
        825 => ("Sharkoon Skiller SGK25 RGB", &L75),
        497 => ("Machenike K500A-B84W", &L84),
        280 | 40 => ("Machenike K500-B94W", &L94),
        484 => ("GameStop 60%", &L60),
        _ => return None,
    })
}

/// The model id in a firmware reply: byte 6 times 255 plus byte 7, as the plugin reads it.
pub fn model_id(reply: &[u8]) -> Option<u32> {
    (reply.len() >= QUERY_REPLY).then(|| reply[6] as u32 * 255 + reply[7] as u32)
}

/// One feature report with every key: three bytes per wire slot after the header.
pub fn colors_report(layout: &Layout, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0u8; COLORS_LEN];
    p[..4].copy_from_slice(&COLORS_HEAD);
    for (&slot, c) in layout.slots.iter().zip(colors) {
        let at = 4 + slot as usize * 3;
        p[at..at + 3].copy_from_slice(&rgb(*c));
    }
    p
}

fn query(io: &mut dyn Io) -> Result<u32, String> {
    io.feature(&QUERY, QUERY.len())?;
    io.sleep(1);
    let r = io.get_feature(QUERY[0], QUERY_REPLY)?;
    model_id(&r).ok_or_else(|| "short firmware reply".into())
}

struct Sino(&'static Layout);

impl Proto for Sino {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&colors_report(self.0, colors), COLORS_LEN)?;
        io.sleep(1);
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(&colors_report(self.0, &[]), COLORS_LEN);
    }
}

fn open(api: &HidApi, info: &Info, _: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = Collections::open(api, info)?;
    let id = query(&mut io)?;
    let (name, layout) = model(id).ok_or_else(|| format!("unknown Sinowealth model {id}"))?;
    let dev = positioned(
        &info.device_id(FAMILY),
        name,
        DeviceKind::Keyboard,
        layout.w,
        layout.h,
        layout.pos,
    );
    Ok(Driver::new(Box::new(io), dev, FAMILY, Sino(layout)).boxed())
}

pub static DETECTORS: &[Detector] = &[vendor("Sinowealth Keyboard", 0x258A, 0x0049, open)];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::super::rk::check;
    use super::*;

    #[test]
    fn layouts_fit() {
        use layouts::*;
        for l in [&FULL, &FULLV2, &L75, &L68, &L68_V2, &L60, &L96, &L94, &L84] {
            check(l, COLORS_LEN - 4, 3);
        }
    }

    #[test]
    fn query_reads_model() {
        let mut io = Rec::default();
        io.replies.push_back(vec![0x05, 0, 0, 0, 0, 0, 51, 54]);
        assert_eq!(query(&mut io), Ok(51 * 255 + 54));
        assert_eq!(io.ops[0], Op::Feature(QUERY.to_vec()));
        assert_eq!(io.ops[1], Op::GetFeature(0x05, 8));
        assert_eq!(model(13059).unwrap().0, "Redragon K618 Horus");
        assert!(model(7).is_none());
    }

    #[test]
    fn colors_go_to_slot_times_three() {
        let mut c = vec![Color(0, 0, 0); layouts::FULL.slots.len()];
        c[1] = Color(9, 8, 7);
        let p = colors_report(&layouts::FULL, &c);
        assert_eq!(p.len(), 382);
        assert_eq!(p[..4], [0x08, 0x0A, 0x7A, 0x01]);
        let slot = layouts::FULL.slots[1] as usize;
        assert_eq!(p[4 + slot * 3..7 + slot * 3], [9, 8, 7]);
    }
}
