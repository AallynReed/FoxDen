use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x248A;
const MW400: u16 = 0xFA02;
const REPORT: u8 = 0x05;

fn vendor_collection(c: &Coll) -> bool {
    c.iface == 0 && c.page == 0x0001 && c.usage == 0x0000 && c.in_len == 33 && c.out_len == 33
}

/// Battery reply `[05][..][FA][48][raw lo][raw hi][charging]`.
fn parse_reply(r: &[u8]) -> Option<(u16, bool)> {
    (r.len() >= 7 && r[0] == REPORT && r[2] == 0xFA && r[3] == 0x48)
        .then(|| (u16::from_le_bytes([r[4], r[5]]), r[6] != 0))
}

fn millivolts(raw: u16) -> u32 {
    raw as u32 * 990 / 240
}

/// The vendor app's 10% steps.
fn percent(mv: u32) -> u8 {
    const STEPS: [(u32, u8); 9] = [
        (4200, 100),
        (4100, 90),
        (4000, 80),
        (3900, 70),
        (3800, 60),
        (3700, 50),
        (3500, 40),
        (3400, 30),
        (3200, 20),
    ];
    STEPS.iter().find(|(v, _)| mv >= *v).map_or(10, |s| s.1)
}

pub fn poll_niceboy(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, MW400) {
        return;
    }
    for c in ctx.find(VID, MW400, vendor_collection) {
        let Some(dev) = Dev::open(&c) else { continue };
        dev.flush();
        if !dev.write(&[REPORT, 0x24, 0xFA, 0x48]) {
            continue;
        }
        let Some((raw, charging)) = dev.read_until(Duration::from_millis(500), parse_reply) else {
            continue;
        };
        let mut b = Battery {
            key: ctx.stable_key(&format!("niceboy-{VID:04X}:{MW400:04X}")),
            name: "Niceboy MW400".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            grace_secs: 1800,
            ..Default::default()
        };
        if raw == 0 && !charging {
            b.asleep = true;
        } else {
            b.millivolts = millivolts(raw);
            b.percent = Some(percent(b.millivolts));
            b.charging = charging;
        }
        ctx.add(b, VID, MW400);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_to_millivolts_and_percent() {
        let (raw, chg) = parse_reply(&[0x05, 0x49, 0xFA, 0x48, 0xEA, 0x03, 0x00]).unwrap();
        assert_eq!((raw, chg), (1002, false));
        assert_eq!(millivolts(raw), 4133);
        assert_eq!(percent(4133), 90);
        assert_eq!(percent(3100), 10);
        assert_eq!(
            parse_reply(&[0x05, 0x49, 0xFA, 0x47, 0xEA, 0x03, 0x00]),
            None
        );
    }
}
