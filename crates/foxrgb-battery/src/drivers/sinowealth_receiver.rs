use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x258A;
const PID: u16 = 0x002F;
const REPORT: u8 = 0x05;
const LINK: u8 = 0x80;
const LEVEL: u8 = 0x90;

/// Reply to `cmd`: `[05][cmd][a][b]..`, 8 bytes; returns (a, b).
fn parse_reply(r: &[u8], cmd: u8) -> Option<(u8, u8)> {
    (r.len() == 8 && r[0] == REPORT && r[1] == cmd).then(|| (r[2], r[3]))
}

fn query(dev: &Dev, cmd: u8) -> Option<(u8, u8)> {
    if !dev.set_feature(&[REPORT, cmd, 0, 0, 0, 0, 0, 0]) {
        return None;
    }
    sleep(Duration::from_millis(100));
    parse_reply(&dev.get_feature(REPORT)?, cmd)
}

fn read_level(dev: &Dev) -> Option<u8> {
    let (linked, _) = query(dev, LINK)?;
    if linked != 1 {
        return None;
    }
    match query(dev, LEVEL)? {
        (0x11, pct) if pct <= 100 => Some(pct),
        _ => None,
    }
}

pub fn poll_sinowealth_receiver(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.iface == 1
            && c.page == 0xFF00
            && c.usage == 1
            && c.feat_len == 8
            && c.in_len == 0
            && c.out_len == 0
    });
    for c in colls {
        let Some(pct) = Dev::open_feature_only(&c).and_then(|d| read_level(&d)) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("sinowealth-{VID:04X}:{PID:04X}")),
            name: "Sinowealth wireless mouse".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, VID, PID);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_status_reports() {
        assert_eq!(
            parse_reply(&[0x05, 0x90, 0x11, 0x4B, 0, 0, 0, 0], LEVEL),
            Some((0x11, 75))
        );
        assert_eq!(parse_reply(&[0x05, 0x80, 0x01, 0, 0, 0, 0, 0], LEVEL), None);
        assert_eq!(parse_reply(&[0x05, 0x90, 0x11], LEVEL), None);
    }
}
