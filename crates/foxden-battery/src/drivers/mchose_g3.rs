use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3837;
const CABLE: u16 = 0x418C;
const RECEIVER: u16 = 0x4245;
const STATUS: [u8; 11] = [
    0x00, 0x55, 0x30, 0xA5, 0x0B, 0x2E, 0x01, 0x01, 0x01, 0x00, 0x00,
];

/// `None` for other reports; `Some(None)` for a status reply without a valid level.
fn parse(r: &[u8]) -> Option<Option<(u8, bool)>> {
    if r.len() < 10 || r[0] != 0xAA || r[1] != 0x30 {
        return None;
    }
    Some((1..=100).contains(&r[8]).then_some((r[8], r[9] == 1)))
}

fn query(dev: &Dev) -> Option<(u8, bool)> {
    dev.flush();
    if !dev.write(&STATUS) {
        return None;
    }
    dev.read_until(Duration::from_secs(1), parse).flatten()
}

pub fn poll_mchose_g3(ctx: &mut Ctx) {
    // The cable wins when both are plugged in.
    let mut found = false;
    for pid in [CABLE, RECEIVER] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.page == 0xFF01 && c.usage == 0x0010 && c.out_len >= 65 && c.in_len >= 65
        });
        for c in colls {
            if found {
                ctx.claim(VID, pid);
                break;
            }
            let Some((pct, charging)) = Dev::open(&c).and_then(|d| query(&d)) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key("mchose-g3v2"),
                name: if c.product.is_empty() {
                    "MCHOSE G3 V2".into()
                } else {
                    c.product.clone()
                },
                via: if pid == CABLE {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                percent: Some(pct),
                charging,
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, VID, pid);
            found = true;
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reply() {
        let mut r = [0u8; 64];
        r[..10].copy_from_slice(&[0xAA, 0x30, 0, 0, 0, 0, 0, 0, 58, 1]);
        assert_eq!(parse(&r), Some(Some((58, true))));
        r[8] = 0;
        assert_eq!(parse(&r), Some(None));
        r[1] = 0x31;
        assert_eq!(parse(&r), None);
    }
}
