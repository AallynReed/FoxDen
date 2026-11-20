use std::time::{Duration, Instant};

use super::vendor_frames::report8;
use crate::hid::Dev;
use crate::util::clamp100;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x25A7;
const CABLE: u16 = 0xFA7B;
const DONGLE: u16 = 0xFA7C;
const INVADER: u16 = 0xFA7E;
const REPORT: u8 = 0x08;
const BATTERY: u8 = 0x04;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    pct: u8,
    charging: bool,
    checked: bool,
}

/// `[08][04][..][..][..][..][percent][charging]`; `checked` when the 17 bytes sum to 0x55.
fn parse_reply(r: &[u8]) -> Option<Reading> {
    if r.len() < 8 || r[0] != REPORT || r[1] != BATTERY || !(1..=100).contains(&r[6]) || r[7] > 1 {
        return None;
    }
    let checked = r.len() >= 17 && r[..17].iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0x55;
    Some(Reading {
        pct: r[6],
        charging: r[7] == 1,
        checked,
    })
}

fn better(new: Reading, old: Option<Reading>) -> Option<Reading> {
    match old {
        Some(o) if o.checked || !new.checked => Some(o),
        _ => Some(new),
    }
}

/// Asks on `cmd` (feature report) and listens on every reader: the answer comes back on a sibling collection.
fn query(cmd: &Dev, readers: &[&Dev]) -> Option<Reading> {
    cmd.flush();
    if !cmd.set_feature(&report8(BATTERY)) {
        return None;
    }
    let end = Instant::now() + Duration::from_millis(500);
    let mut found = None;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        for d in readers {
            if let Some(r) = d.read(left.min(Duration::from_millis(40)))
                && let Some(new) = parse_reply(&r)
            {
                found = better(new, found);
            }
        }
        if found.is_some_and(|f| f.checked) || readers.is_empty() {
            break;
        }
    }
    if let Some(new) = cmd.get_feature(REPORT).as_deref().and_then(parse_reply) {
        found = better(new, found);
    }
    found
}

pub fn poll_fantech(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        if ![CABLE, DONGLE, INVADER].contains(&pid) {
            continue;
        }
        let all = ctx.find(vid, pid, |_| true);
        let Some(feat) = all.iter().find(|c| c.feat_len >= 17) else {
            continue;
        };
        let Some(cmd) = Dev::open(feat).or_else(|| Dev::open_feature_only(feat)) else {
            continue;
        };
        let others: Vec<Dev> = all
            .iter()
            .filter(|c| c.path != feat.path && c.in_len >= 6)
            .filter_map(Dev::open)
            .collect();
        let mut readers: Vec<&Dev> = others.iter().collect();
        if feat.in_len >= 6 {
            readers.push(&cmd);
        }
        let Some(r) = query(&cmd, &readers) else {
            continue;
        };
        let invader = pid == INVADER;
        let b = Battery {
            key: ctx.stable_key(&format!(
                "{}-{vid:04X}:{pid:04X}",
                if invader { "redragon" } else { "fantech" }
            )),
            name: if invader {
                "Redragon Invader Pro"
            } else {
                "Fantech Helios Go XD5"
            }
            .into(),
            via: if pid == CABLE {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Mouse,
            // The Invader reports tenths.
            percent: Some(if invader {
                clamp100(r.pct as i32 * 10)
            } else {
                r.pct
            }),
            charging: r.charging,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, vid, pid);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_reply_prefers_a_valid_checksum() {
        let mut r = vec![0u8; 17];
        r[..8].copy_from_slice(&[0x08, 0x04, 0, 0, 0, 0, 77, 1]);
        let sum = r[..16].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        r[16] = 0x55u8.wrapping_sub(sum);
        let good = parse_reply(&r).unwrap();
        assert_eq!(
            good,
            Reading {
                pct: 77,
                charging: true,
                checked: true
            }
        );
        r[16] ^= 1;
        let loose = parse_reply(&r).unwrap();
        assert!(!loose.checked);
        assert_eq!(better(loose, Some(good)), Some(good));
        assert_eq!(better(good, Some(loose)), Some(good));
        r[1] = 0x07;
        assert_eq!(parse_reply(&r), None);
    }
}
