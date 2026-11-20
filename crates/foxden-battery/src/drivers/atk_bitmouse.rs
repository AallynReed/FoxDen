use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x373B;
const NK_DONGLE: u16 = 0x1216;
const NANO_8K: u16 = 0x1178;
const ZERO_CABLE: u16 = 0x1154;
const ZERO_RECEIVERS: [u16; 2] = [0x124F, 0x1155];

const LEVEL: u8 = 0x07;
const ZERO_CHARGE: u8 = 0x08;
const NK_CHARGE: u8 = 0x0F;

#[derive(Clone, Copy, PartialEq)]
enum Route {
    NearLink,
    ZeroCable,
    ZeroReceiver,
}

impl Route {
    fn zero(self) -> bool {
        self != Route::NearLink
    }
}

/// Report 08 `[08][check][72][02][00][target][cmd][01]`, 64 bytes; target 00 = the device itself, 01 = behind the dongle.
fn request(cmd: u8, link: Route) -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = 0x08;
    r[2] = 0x72;
    r[3] = 0x02;
    r[5] = u8::from(link != Route::ZeroCable);
    r[6] = cmd;
    r[7] = 0x01;
    r[1] = if link.zero() {
        0x7D
    } else {
        r[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b))
    };
    r
}

/// Reply `[08][72][status][seq][00][cmd][01][value]`: `Some(Ok(value))`, `Some(Err(()))` on an error status,
/// `None` for anything else.
fn parse_reply(r: &[u8], cmd: u8, link: Route) -> Option<Result<u8, ()>> {
    if r.len() < 8 || r[0] != 0x08 || r[1] != 0x72 || r[5] != cmd || (link.zero() && r[6] != 0x01) {
        return None;
    }
    // Zero links only trust a fresh (00) status; NearLink flags errors with FF.
    let failed = if link.zero() {
        r[2] != 0x00
    } else {
        r[2] == 0xFF
    };
    Some(if failed { Err(()) } else { Ok(r[7]) })
}

fn query(dev: &Dev, cmd: u8, link: Route) -> Option<u8> {
    dev.flush();
    if !dev.write(&request(cmd, link)) {
        return None;
    }
    let wait = Duration::from_millis(if link.zero() { 500 } else { 700 });
    dev.read_until(wait, |r| parse_reply(r, cmd, link))?.ok()
}

fn poll_zero(ctx: &mut Ctx) {
    let mut reported = false;
    for pid in [ZERO_CABLE, ZERO_RECEIVERS[0], ZERO_RECEIVERS[1]] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.page == 0xFF05 && c.usage == 0x0001 && c.out_len == 64 && c.in_len == 64
        });
        for c in colls {
            // Claimed even when asleep, so the report 08 mouse protocol never reaches it.
            ctx.claim(VID, pid);
            if reported {
                break;
            }
            let Some(dev) = Dev::open(&c) else { continue };
            let cable = pid == ZERO_CABLE;
            let link = if cable {
                Route::ZeroCable
            } else {
                Route::ZeroReceiver
            };
            let Some(pct) = query(&dev, LEVEL, link).filter(|&p| p <= 100) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key("atk-zero"),
                name: "ATK Zero".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                percent: Some(pct),
                charging: cable && query(&dev, ZERO_CHARGE, link) == Some(0x02),
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, VID, pid);
            reported = true;
            break;
        }
    }
}

/// GetConfigData reply `[82][rate][dpi][light][brightness][speed][color][..][battery][link]`:
/// `Some(None)` when no mouse is linked or the level is out of range.
fn parse_config(r: &[u8]) -> Option<Option<(u8, bool)>> {
    if r.len() < 10 || r[0] != 0x82 {
        return None;
    }
    let pct = r[8] & 0x7F;
    Some((r[9] != 0 && (1..=100).contains(&pct)).then_some((pct, r[8] & 0x80 != 0)))
}

fn nano_8k_collection(c: &Coll) -> bool {
    c.page == 0xFF10 && c.usage == 0x0001 && c.feat_len >= 9 && c.in_len >= 11
}

fn poll_nano_8k(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, NANO_8K) {
        return;
    }
    for c in ctx.find(VID, NANO_8K, nano_8k_collection) {
        let Some(dev) = Dev::open(&c) else { continue };
        dev.flush();
        // GetConfigData: a read request, sent as feature report 0.
        if !dev.set_feature(&[0x00, 0x82]) {
            continue;
        }
        let Some(config) = dev.read_until(Duration::from_millis(700), parse_config) else {
            continue;
        };
        let Some((pct, charging)) = config else {
            ctx.claim(VID, NANO_8K);
            return;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("atk-{VID:04X}:{NANO_8K:04X}")),
            name: "ATK 8K Nano mouse".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            charging,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, NANO_8K);
        return;
    }
}

pub fn poll_atk_bitmouse(ctx: &mut Ctx) {
    poll_zero(ctx);
    poll_nano_8k(ctx);
    if ctx.is_claimed(VID, NK_DONGLE) {
        return;
    }
    let colls = ctx.find(VID, NK_DONGLE, |c| {
        c.page == 0xFF05 && c.usage == 0x0001 && c.out_len >= 64 && c.in_len >= 8
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        let Some(pct) = query(&dev, LEVEL, Route::NearLink).filter(|p| (1..=100).contains(p))
        else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("atk-{VID:04X}:{NK_DONGLE:04X}")),
            name: "ATK / VXE NK mouse".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            charging: query(&dev, NK_CHARGE, Route::NearLink) == Some(1),
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, NK_DONGLE);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_per_link() {
        let nk = request(LEVEL, Route::NearLink);
        assert_eq!(nk[..8], [0x08, 0x7D, 0x72, 0x02, 0x00, 0x01, 0x07, 0x01]);
        assert_eq!(request(NK_CHARGE, Route::NearLink)[1], 0x85);
        let cable = request(LEVEL, Route::ZeroCable);
        assert_eq!(cable[..8], [0x08, 0x7D, 0x72, 0x02, 0x00, 0x00, 0x07, 0x01]);
        assert_eq!(request(LEVEL, Route::ZeroReceiver)[5], 0x01);
    }

    #[test]
    fn replies_per_link() {
        let ok = [0x08, 0x72, 0x00, 0x05, 0x00, 0x07, 0x01, 0x42];
        assert_eq!(parse_reply(&ok, LEVEL, Route::ZeroCable), Some(Ok(0x42)));
        assert_eq!(parse_reply(&ok, ZERO_CHARGE, Route::ZeroCable), None);
        let stale = [0x08, 0x72, 0x01, 0x05, 0x00, 0x07, 0x01, 0x42];
        assert_eq!(
            parse_reply(&stale, LEVEL, Route::ZeroReceiver),
            Some(Err(()))
        );
        assert_eq!(parse_reply(&stale, LEVEL, Route::NearLink), Some(Ok(0x42)));
        let err = [0x08, 0x72, 0xFF, 0x05, 0x00, 0x07, 0x00, 0x00];
        assert_eq!(parse_reply(&err, LEVEL, Route::NearLink), Some(Err(())));
    }

    #[test]
    fn nano_8k_config() {
        let mut r = vec![0u8; 64];
        r[0] = 0x82;
        r[8] = 0x80 | 66;
        r[9] = 1;
        assert_eq!(parse_config(&r), Some(Some((66, true))));
        r[9] = 0;
        assert_eq!(parse_config(&r), Some(None));
        r[0] = 0x12;
        assert_eq!(parse_config(&r), None);
    }
}
