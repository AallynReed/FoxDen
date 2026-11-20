use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x057E;
const PID: u16 = 0x2009;
const LEVELS: [&str; 5] = ["Empty", "Critical", "Low", "Medium", "Full"];

/// Full input report 30: byte 2 bits 7..5 = level 0..4, bit 4 = charging.
fn parse(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 13 || r[0] != 0x30 {
        return None;
    }
    let level = r[2] >> 5;
    (level <= 4).then_some((level, r[2] & 0x10 != 0 && level < 4))
}

pub fn poll_switch_pro(ctx: &mut Ctx) {
    let colls = ctx.find(VID, PID, |c| {
        c.page == 0x01 && matches!(c.usage, 0x04 | 0x05) && c.in_len >= 13
    });
    for c in colls {
        let Some(d) = Dev::open_read_only(&c) else {
            continue;
        };
        let end = Instant::now() + Duration::from_millis(350);
        let mut got = None;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let Some(r) = d.read(left) else { break };
            if let Some(v) = parse(&r) {
                got = Some(v);
                break;
            }
        }
        let Some((level, charging)) = got else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("switchpro-{}:{}", hex4(VID), hex4(PID))),
            name: if c.product.is_empty() {
                "Nintendo Switch Pro Controller".into()
            } else {
                c.product.clone()
            },
            via: "Switch mode".into(),
            kind: Kind::Gamepad,
            percent: Some(level * 25),
            level: LEVELS[level as usize].into(),
            charging,
            grace_secs: 300,
            ..Battery::default()
        };
        ctx.add(b, VID, PID);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_and_charging_bits() {
        let mut r = [0u8; 13];
        r[0] = 0x30;
        r[2] = 0b0111_0000;
        assert_eq!(parse(&r), Some((3, true)));
        r[2] = 0b1001_0000;
        assert_eq!(parse(&r), Some((4, false)));
        r[2] = 0b1010_0000;
        assert_eq!(parse(&r), None);
        r[0] = 0x21;
        assert_eq!(parse(&r), None);
    }
}
