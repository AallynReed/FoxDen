use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x19F5;
const WH80: u16 = 0xA011;

/// GetFunc 05 at offset 0x0020, one byte.
const REQUEST: [u8; 9] = [0x00, 0x55, 0x05, 0x00, 0x21, 0x01, 0x20, 0x00, 0x00];

/// `(percent, charging)`; the reply checksum at [3] is the low byte of the sum of bytes 4..
fn parse_battery(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() != 64 || r[..3] != [0xAA, 0x05, 0x00] || r[4..8] != [0x01, 0x20, 0x00, 0x00] {
        return None;
    }
    let sum = r[4..].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    let pct = r[8] & 0x7F;
    (sum == r[3] && pct <= 100).then_some((pct, r[8] & 0x80 != 0))
}

pub fn poll_nuphy(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, WH80) {
        return;
    }
    let colls = ctx.find(VID, WH80, |c| {
        c.iface == 1 && c.page == 0x01 && c.usage == 0 && c.in_len == 65 && c.out_len == 65
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        dev.flush();
        if !dev.write_timeout(&REQUEST, Duration::from_millis(400)) {
            continue;
        }
        let Some((percent, charging)) = dev.read_until(Duration::from_millis(400), parse_battery)
        else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key("nuphy-WH80"),
            name: "NuPhy WH80".into(),
            via: "USB cable".into(),
            kind: Kind::Keyboard,
            percent: Some(percent),
            charging,
            ..Default::default()
        };
        ctx.add(b, VID, WH80);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(value: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..8].copy_from_slice(&[0xAA, 0x05, 0x00, 0x00, 0x01, 0x20, 0x00, 0x00]);
        r[8] = value;
        r[3] = r[4..].iter().fold(0u8, |s, &b| s.wrapping_add(b));
        r
    }

    #[test]
    fn battery_byte_splits_charging_and_percent() {
        assert_eq!(parse_battery(&reply(0x80 | 70)), Some((70, true)));
        assert_eq!(parse_battery(&reply(30)), Some((30, false)));
        let mut bad = reply(30);
        bad[3] ^= 1;
        assert_eq!(parse_battery(&bad), None);
    }
}
