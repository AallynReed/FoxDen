use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x35D8;
const GM01_PRO: u16 = 0x0133;
const REPORT: u8 = 0x4B;
const READ: u8 = 0x80;
const BATTERY: u8 = 0x27;
const LINK: u8 = 0x31;

/// Reply `[4B][80][cmd][len][data..]`.
fn is_reply(r: &[u8], cmd: u8) -> bool {
    r.len() >= 5 && r[0] == REPORT && r[1] == READ && r[2] == cmd
}

/// Battery data `[00 00 00 00][link][percent]`; only while the headset is linked.
fn parse_battery(r: &[u8]) -> Option<u8> {
    (r.len() >= 10 && r[3] >= 6 && r[8] != 0 && r[9] <= 100).then(|| r[9])
}

/// Link data `[linked]`, answered by the dongle itself.
fn unlinked(r: &[u8]) -> bool {
    r[3] >= 1 && r[4] == 0
}

fn query(dev: &Dev, cmd: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&[REPORT, READ, cmd, 0x00]) {
        return None;
    }
    dev.read_until(Duration::from_millis(400), |r| {
        is_reply(r, cmd).then(|| r.to_vec())
    })
}

pub fn poll_moondrop(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, GM01_PRO) {
        return;
    }
    let colls = ctx.find(VID, GM01_PRO, |c| c.iface == 3 && c.page == 0x000C);
    if colls.is_empty() {
        return;
    }
    let mut b = Battery {
        key: ctx.stable_key(&format!("moondrop-{VID:04X}:{GM01_PRO:04X}")),
        name: "MOONDROP GM01 Pro".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Headset,
        grace_secs: 1800,
        ..Default::default()
    };
    let mut dongle_unlinked = false;
    // Only the dongle has the control channel.
    for c in colls.iter().filter(|c| c.out_len >= 4 && c.in_len >= 10) {
        let Some(dev) = Dev::open(c) else { continue };
        if let Some(pct) = query(&dev, BATTERY).as_deref().and_then(parse_battery) {
            b.percent = Some(pct);
            ctx.add(b, VID, GM01_PRO);
            return;
        }
        dongle_unlinked |= query(&dev, LINK).is_some_and(|r| unlinked(&r));
    }
    // On its cable the headset has one collection there, so Windows adds no &colNN to the path.
    let on_cable = colls
        .iter()
        .any(|c| !c.path.to_lowercase().contains("&col"));
    if on_cable {
        b.via = "USB cable".into();
        b.charging = true;
        ctx.add(b, VID, GM01_PRO);
    } else if dongle_unlinked {
        b.connected = false;
        ctx.add(b, VID, GM01_PRO);
    } else {
        ctx.claim(VID, GM01_PRO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_and_link_replies() {
        let r = [0x4B, 0x80, 0x27, 0x06, 0, 0, 0, 0, 0x01, 0x32];
        assert!(is_reply(&r, BATTERY));
        assert_eq!(parse_battery(&r), Some(50));
        assert_eq!(
            parse_battery(&[0x4B, 0x80, 0x27, 0x06, 0, 0, 0, 0, 0x00, 0x32]),
            None
        );
        assert!(unlinked(&[0x4B, 0x80, 0x31, 0x01, 0x00]));
        assert!(!unlinked(&[0x4B, 0x80, 0x31, 0x01, 0x01]));
    }
}
