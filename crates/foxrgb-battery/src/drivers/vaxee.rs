use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3057;
const RECEIVER_4K: u16 = 0x1002;
const RECEIVER_8K: u16 = 0x2002;
const ZYGEN_CABLE: u16 = 0x1011;

const CMD_BATTERY: u8 = 0x0B;
const CMD_CHARGING: u8 = 0x10;

/// Value byte of a feature report 0E read reply for `cmd`.
fn parse_read(r: &[u8], cmd: u8) -> Option<u8> {
    (r.len() >= 6 && r[0] == 0x0E && r[1] == 0xA5 && r[2] == cmd && r[4] == 1).then_some(r[5])
}

fn read(dev: &Dev, cmd: u8) -> Option<u8> {
    let mut req = [0u8; 64];
    // [3] = 01: read, never 02 (write)
    req[..5].copy_from_slice(&[0x0E, 0xA5, cmd, 0x01, 0x01]);
    if !dev.set_feature(&req) {
        return None;
    }
    sleep(Duration::from_millis(100));
    parse_read(&dev.get_feature(0x0E)?, cmd)
}

/// Battery comes in steps of 0..=20.
fn percent(steps: u8) -> Option<u8> {
    (steps <= 20).then_some(steps * 5)
}

pub fn poll_vaxee(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        if ![RECEIVER_4K, RECEIVER_8K, ZYGEN_CABLE].contains(&pid) {
            continue;
        }
        for c in ctx.find(vid, pid, |c| {
            c.page == 0xFF05 && c.usage == 1 && c.feat_len == 64
        }) {
            let Some(dev) = Dev::open_feature_only(&c) else {
                continue;
            };
            let Some(pct) = (0..2).find_map(|_| read(&dev, CMD_BATTERY).and_then(percent)) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&format!("vaxee-3057:{pid:04X}")),
                name: match pid {
                    ZYGEN_CABLE => "ZYGEN NP-01S V2 Wireless",
                    RECEIVER_8K => "VAXEE 8K Wireless",
                    _ => "VAXEE wireless mouse",
                }
                .into(),
                via: if pid == ZYGEN_CABLE {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                percent: Some(pct),
                charging: read(&dev, CMD_CHARGING).is_some_and(|v| v != 0),
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_steps_of_five() {
        let mut r = [0u8; 64];
        r[..6].copy_from_slice(&[0x0E, 0xA5, CMD_BATTERY, 0x01, 0x01, 17]);
        assert_eq!(parse_read(&r, CMD_BATTERY).and_then(percent), Some(85));
        assert_eq!(parse_read(&r, CMD_CHARGING), None);
        assert_eq!(percent(21), None);
    }
}
