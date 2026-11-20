use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x28DE;
const LEGACY_RECEIVER: u16 = 0x1142;
const PUCK: u16 = 0x1304;

fn transport(pid: u16) -> Option<&'static str> {
    match pid {
        LEGACY_RECEIVER | PUCK | 0x1305 => Some("Wireless adapter"),
        0x1302 => Some("USB cable"),
        0x1303 => Some("Bluetooth"),
        _ => None,
    }
}

/// Legacy status report `01 00 04 0B ... [14] percent`, or the newer `43 [charge state] [percent]`.
fn parse(legacy: bool, r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 15 {
        return None;
    }
    if legacy {
        (r[..4] == [0x01, 0x00, 0x04, 11] && r[14] <= 100).then_some((r[14], false))
    } else {
        (r[0] == 0x43 && r[2] <= 100).then_some((r[2], r[1] == 2))
    }
}

fn read_battery(d: &Dev, legacy: bool, puck: bool) -> Option<(u8, bool)> {
    let end = Instant::now() + Duration::from_secs(5);
    let mut heard = false;
    while Instant::now() < end {
        // An empty puck slot stays silent.
        let wait = if puck && !heard { 250 } else { 100 };
        match d.read(Duration::from_millis(wait)) {
            Some(r) => {
                heard = true;
                if let Some(v) = parse(legacy, &r) {
                    return Some(v);
                }
            }
            None if puck && !heard => return None,
            None => {}
        }
    }
    None
}

pub fn poll_steam_controller(ctx: &mut Ctx) {
    let colls: Vec<_> = ctx
        .colls
        .iter()
        .filter(|c| {
            c.vid == VID && c.page == 0xFF00 && c.in_len >= 15 && transport(c.pid).is_some()
        })
        .filter(|c| c.pid != PUCK || c.usage == 0x01)
        .cloned()
        .collect();
    for c in colls {
        let legacy = c.pid == LEGACY_RECEIVER;
        let Some((percent, charging)) =
            Dev::open(&c).and_then(|d| read_battery(&d, legacy, c.pid == PUCK))
        else {
            continue;
        };
        let generic = legacy || matches!(c.pid, PUCK | 0x1305) || c.product.is_empty();
        let b = Battery {
            key: ctx.stable_key(&format!("steam-{}:{}", hex4(VID), hex4(c.pid))),
            name: if generic {
                "Steam Controller".into()
            } else {
                c.product.clone()
            },
            via: transport(c.pid).unwrap_or_default().into(),
            kind: Kind::Gamepad,
            percent: Some(percent),
            charging,
            ..Battery::default()
        };
        ctx.add(b, VID, c.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_report_layouts() {
        let mut r = [0u8; 64];
        r[..4].copy_from_slice(&[0x01, 0x00, 0x04, 11]);
        r[14] = 62;
        assert_eq!(parse(true, &r), Some((62, false)));
        assert_eq!(parse(false, &r), None);
        let mut n = [0u8; 64];
        n[..3].copy_from_slice(&[0x43, 2, 90]);
        assert_eq!(parse(false, &n), Some((90, true)));
        n[2] = 101;
        assert_eq!(parse(false, &n), None);
    }
}
