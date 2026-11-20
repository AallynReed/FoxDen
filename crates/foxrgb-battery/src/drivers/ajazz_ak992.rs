use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x320F;
const PID: u16 = 0x50C3;
const REPORT: u8 = 0x04;
const BATTERY: u8 = 0x1A;

/// (percent, charging) from a report 04 reply that echoes request `seq`.
fn parse(r: &[u8], seq: u16) -> Option<(u8, bool)> {
    let [lo, hi] = seq.to_le_bytes();
    if r.len() < 10
        || r[..4] != [REPORT, lo, hi, BATTERY]
        || r[5] != 0
        || r[6] != 0
        || r[8] > 100
        || r[9] > 2
    {
        return None;
    }
    Some((r[8], r[9] != 0))
}

fn query(dev: &Dev) -> Option<(u8, bool)> {
    let seq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.subsec_micros() as u16);
    let [lo, hi] = seq.to_le_bytes();
    dev.flush();
    if !dev.write_timeout(&[REPORT, lo, hi, BATTERY], Duration::from_millis(300)) {
        return None;
    }
    dev.read_until(Duration::from_millis(600), |r| parse(r, seq))
}

pub fn poll_ajazz_ak992(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.iface == 1
            && c.page == 0xFF1C
            && c.usage == 0x0092
            && c.in_len == 64
            && c.out_len == 64
            && c.feat_len == 0
    });
    for c in colls {
        let Some((pct, charging)) = Dev::open(&c).and_then(|d| query(&d)) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key("ajazz-320F:50C3"),
            name: "AJAZZ AK992".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Keyboard,
            percent: Some(pct),
            charging,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, PID);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_must_echo_sequence() {
        let r = [0x04, 0x34, 0x12, 0x1A, 0x00, 0x00, 0x00, 0x00, 73, 1, 0];
        assert_eq!(parse(&r, 0x1234), Some((73, true)));
        assert_eq!(parse(&r, 0x1235), None);
        let mut bad = r;
        bad[8] = 101;
        assert_eq!(parse(&bad, 0x1234), None);
    }
}
