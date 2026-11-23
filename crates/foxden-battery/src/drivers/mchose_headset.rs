use std::time::Duration;

use crate::hid::Dev;
use crate::util::xor8;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3837;
const V9_PRO_2: u16 = 0x6011;
const REPORT: u8 = 0x55;
const START: u8 = 0xAA;
const VERSION: u8 = 0x01;
const READ_BATTERY: u8 = 0x0B;
const BATTERY_PUSH: u8 = 0x22;

/// start, version, flags 0 (no checksum), length 1, type 00 01 = request, sub-command, checksum 0.
const REQUEST: [u8; 9] = [
    REPORT,
    START,
    VERSION,
    0x00,
    0x01,
    0x00,
    0x01,
    READ_BATTERY,
    0x00,
];

#[derive(Debug, PartialEq)]
enum Reply {
    Off,
    Level(u8, bool),
}

/// A battery reply or the headset's own battery push; `[0]` is the report id.
fn parse(r: &[u8]) -> Option<Reply> {
    let f = r.strip_prefix(&[REPORT])?;
    if f.len() < 8 || f[0] != START || f[1] != VERSION || f[4] != 0x00 {
        return None;
    }
    if !matches!((f[5], f[6]), (0x01, READ_BATTERY) | (0x04, BATTERY_PUSH)) {
        return None;
    }
    let len = f[3] as usize;
    if len < 3 || 6 + len >= f.len() {
        return None;
    }
    let check = if f[2] == 1 { xor8(&f[2..6 + len]) } else { 0 };
    if f[6 + len] != check {
        return None;
    }
    let body = &f[7..];
    if body[0] == 0 {
        return Some(Reply::Off);
    }
    let pct = body[1] & 0x7F;
    (pct <= 100).then_some(Reply::Level(pct, body[1] & 0x80 != 0))
}

fn query(dev: &Dev) -> Option<Reply> {
    dev.flush();
    if !dev.write(&REQUEST) {
        return None;
    }
    dev.read_until(Duration::from_secs(1), parse)
}

pub fn poll_mchose_headset(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, V9_PRO_2) {
        return;
    }
    let colls = ctx.find(VID, V9_PRO_2, |c| {
        c.page == 0xFF07 && c.usage == 0x0222 && c.out_len >= 64 && c.in_len >= 64
    });
    for c in colls {
        let Some(reply) = Dev::open(&c).and_then(|d| query(&d)) else {
            continue;
        };
        let mut b = Battery {
            key: ctx.stable_key("mchose-v9pro2"),
            name: if c.product.is_empty() {
                "MCHOSE V9 Pro 2".into()
            } else {
                c.product.clone()
            },
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            grace_secs: 1800,
            ..Default::default()
        };
        match reply {
            Reply::Level(pct, charging) => {
                b.percent = Some(pct);
                b.charging = charging;
            }
            Reply::Off => b.asleep = true,
        }
        ctx.add(b, VID, V9_PRO_2);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_reply_without_checksum() {
        let r = [
            REPORT,
            START,
            VERSION,
            0x00,
            0x03,
            0x00,
            0x01,
            READ_BATTERY,
            0x01,
            0x80 | 64,
            0x00,
            0,
            0,
        ];
        assert_eq!(parse(&r), Some(Reply::Level(64, true)));
    }

    #[test]
    fn push_with_xor_checksum() {
        let mut r = vec![
            REPORT,
            START,
            VERSION,
            0x01,
            0x03,
            0x00,
            0x04,
            BATTERY_PUSH,
            0x01,
            45,
        ];
        let c = xor8(&r[3..10]);
        r.push(c);
        r.resize(63, 0);
        assert_eq!(parse(&r), Some(Reply::Level(45, false)));
        r[10] ^= 0xFF;
        assert_eq!(parse(&r), None);
    }

    #[test]
    fn headset_off() {
        let r = [
            REPORT,
            START,
            VERSION,
            0x00,
            0x03,
            0x00,
            0x01,
            READ_BATTERY,
            0x00,
            0x00,
            0x00,
            0,
        ];
        assert_eq!(parse(&r), Some(Reply::Off));
    }
}
