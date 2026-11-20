use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x372E;
const ZHOU_DONGLE: u16 = 0x10BB;
const REPORT: u8 = 0x55;
const BATTERY: u8 = 0x65;

/// Percent from a battery reply, numbered or not. No charging flag exists.
fn parse(r: &[u8]) -> Option<u8> {
    let pct = match r {
        [REPORT, BATTERY, pct, ..] => *pct,
        [BATTERY, pct, ..] => *pct,
        _ => return None,
    };
    (pct <= 100).then_some(pct)
}

fn query(dev: &Dev) -> Option<u8> {
    dev.flush();
    // No interrupt OUT pipe on this collection.
    let first = dev.set_output_and_read(&[REPORT, BATTERY, 0x01], Duration::from_millis(500))?;
    // The reply can queue behind other notifications.
    parse(&first).or_else(|| dev.read_until(Duration::from_millis(300), parse))
}

pub fn poll_vgn(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, ZHOU_DONGLE) {
        return;
    }
    let colls = ctx.find(VID, ZHOU_DONGLE, |c| {
        c.page == 0x000C && c.usage == 0x0001 && c.out_len >= 63 && c.in_len >= 3
    });
    for c in colls {
        let Some(pct) = Dev::open(&c).and_then(|d| query(&d)) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key("vgn-zhou"),
            name: if c.product.is_empty() {
                "VGN OMNISOUND (ZHOUTING)".into()
            } else {
                c.product.clone()
            },
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            percent: Some(pct),
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, ZHOU_DONGLE);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_and_bare_replies() {
        assert_eq!(parse(&[0x55, 0x65, 72, 0]), Some(72));
        assert_eq!(parse(&[0x65, 40]), Some(40));
        assert_eq!(parse(&[0x55, 0x65, 200]), None);
        assert_eq!(parse(&[0x55, 0x66, 50]), None);
    }
}
