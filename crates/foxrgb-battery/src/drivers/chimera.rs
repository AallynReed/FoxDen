use std::time::Duration;

use super::vendor_frames::{report8, report8_reply};
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3554;
const V2: u16 = 0xF512;
const ONLINE: u8 = 0x03;
const BATTERY: u8 = 0x04;

#[derive(Debug, PartialEq)]
struct Level {
    pct: u8,
    charging: bool,
    mv: u32,
}

/// Battery reply: `[5]` 02 = linked, `[6]` percent, `[7]` charging, `[8..10]` millivolts big-endian.
fn parse_battery(r: &[u8]) -> Option<Level> {
    if r.len() < 10 || r[5] != 0x02 || !(1..=100).contains(&r[6]) {
        return None;
    }
    Some(Level {
        pct: r[6],
        charging: r[7] != 0,
        mv: u16::from_be_bytes([r[8], r[9]]) as u32,
    })
}

fn query(dev: &Dev, cmd: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&report8(cmd)) {
        return None;
    }
    dev.read_until(Duration::from_millis(800), |r| {
        report8_reply(r, cmd).then(|| r.to_vec())
    })
}

fn read_level(dev: &Dev) -> Option<Level> {
    // The online query also wakes the link.
    if query(dev, ONLINE)?.get(5).is_none_or(|&s| s == 0) {
        return None;
    }
    parse_battery(&query(dev, BATTERY)?)
}

pub fn poll_chimera(ctx: &mut Ctx) {
    if !ctx.products(VID).contains(&(VID, V2)) {
        return;
    }
    let colls = ctx.find(VID, V2, |c| {
        c.page == 0xFF02 && c.usage == 0x0002 && c.in_len >= 17 && c.out_len >= 17
    });
    for c in colls {
        let Some(l) = Dev::open(&c).and_then(|d| read_level(&d)) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("chimera-{VID:04X}:{V2:04X}")),
            name: "Chimera V2".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(l.pct),
            charging: l.charging,
            millivolts: l.mv,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, V2);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_reply() {
        let r = [0x08, 0x04, 0, 0, 0, 0x02, 88, 0x01, 0x0F, 0xD2];
        assert_eq!(
            parse_battery(&r),
            Some(Level {
                pct: 88,
                charging: true,
                mv: 4050
            })
        );
        let off = [0x08, 0x04, 0, 0, 0, 0x01, 88, 0x01, 0x0F, 0xD2];
        assert_eq!(parse_battery(&off), None);
    }
}
