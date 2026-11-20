//! BEITONG KP20 and KP40D pads on their 2.4 GHz receivers: the vendor "base info" request on the
//! FF00:0003 collection. Every byte after the report id goes through a fixed nibble cipher.

use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x20BC;
const KP20: u16 = 0x5128;
const KP40D: u16 = 0x515C;
const REPORT: u8 = 0x02;
const BASE_INFO: u8 = 0x15;

const NIBBLE: [u8; 16] = [
    0x0, 0xA, 0x6, 0xF, 0x1, 0xC, 0x2, 0x9, 0xE, 0x3, 0x7, 0xD, 0x4, 0xB, 0x5, 0x8,
];

fn enc(x: u8) -> u8 {
    (NIBBLE[(x >> 4) as usize] << 4) | ((x & 0x0F) ^ 0x08)
}

fn dec(x: u8) -> u8 {
    let hi = NIBBLE.iter().position(|&n| n == x >> 4).unwrap_or(0) as u8;
    (hi << 4) | ((x & 0x0F) ^ 0x08)
}

fn name(pid: u16) -> Option<&'static str> {
    match pid {
        KP20 => Some("BEITONG KP20"),
        KP40D => Some("BEITONG KP40D"),
        _ => None,
    }
}

/// Level and charging from a base-info reply `02 15 <level> 00 <charge>` (cipher applied).
fn parse(r: &[u8], pid: u16) -> Option<(u8, bool)> {
    if r.len() < 5 || r[0] != REPORT || dec(r[1]) != BASE_INFO {
        return None;
    }
    let level = dec(r[2]);
    if level > 100 {
        return None;
    }
    let mut charge = dec(r[4]);
    if pid == KP40D {
        charge &= 0x0F;
    }
    Some((level, charge == 1))
}

pub fn poll_beitong(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        let Some(model) = name(pid) else { continue };
        let colls = ctx.find(vid, pid, |c| {
            c.page == 0x00FF && c.usage == 0x0003 && c.out_len >= 3 && c.in_len >= 5
        });
        for c in &colls {
            let Some(d) = Dev::open(c) else { continue };
            d.flush();
            if !d.write(&[REPORT, enc(BASE_INFO)]) {
                continue;
            }
            let Some((pct, charging)) = d.read_until(Duration::from_millis(800), |r| parse(r, pid))
            else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&format!("beitong-{vid:04x}:{pid:04x}")),
                name: model.into(),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Gamepad,
                percent: Some(pct),
                charging,
                ..Default::default()
            };
            ctx.add(b, vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cipher_round_trips() {
        assert_eq!(enc(BASE_INFO), 0xAD);
        assert!((0..=255u8).all(|x| dec(enc(x)) == x));
    }

    #[test]
    fn base_info_reply() {
        let r = [REPORT, enc(0x15), enc(73), enc(0), enc(1)];
        assert_eq!(parse(&r, KP20), Some((73, true)));
        let r = [REPORT, enc(0x15), enc(40), enc(0), enc(0x21)];
        assert_eq!(parse(&r, KP20), Some((40, false)));
        assert_eq!(parse(&r, KP40D), Some((40, true)));
        assert_eq!(parse(&[REPORT, enc(0x25), enc(40), 0, 0], KP20), None);
    }
}
