use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1038;
/// Standard and Sage & Gold editions: same base station protocol.
const PIDS: &[u16] = &[0x2244, 0x2270];
const HEADSET_OFF: u8 = 0x01;
const ON_CABLE: u8 = 0x02;

#[derive(Debug, Default, PartialEq)]
struct State {
    level: Option<u8>,
    power: Option<u8>,
    charging: bool,
}

/// Folds one report into the state: the direct `01 B0` reply, or the pushed `07 B7` / `07 B5` frames.
fn absorb(s: &mut State, r: &[u8]) {
    match r {
        [0x01, 0xB0, ..] if r.len() > 15 => {
            if r[6] <= 100 {
                s.level = Some(r[6]);
            }
            s.charging = r[15] == 0x02;
            s.power = Some(r[14]);
        }
        [0x07, 0xB7, level @ 0..=100, _, ch, ..] => {
            s.level = Some(*level);
            s.charging = *ch == 0x02;
        }
        [0x07, 0xB5, _, _, power, ..] => s.power = Some(*power),
        _ => {}
    }
}

pub fn poll_steel_series_elite(ctx: &mut Ctx) {
    for &pid in PIDS {
        if !ctx.is_claimed(VID, pid) {
            poll_base(ctx, pid);
        }
    }
}

fn poll_base(ctx: &mut Ctx, pid: u16) {
    let mut colls = ctx.find(VID, pid, |c| {
        c.iface == 3 && (c.in_len > 0 || c.out_len > 0)
    });
    colls.sort_by_key(|c| match c.page {
        0xFFC0 => 0,
        0xFF00.. => 1,
        _ => 2,
    });
    let devs: Vec<Dev> = colls.iter().filter_map(Dev::open).collect();
    for d in &devs {
        d.flush();
    }
    let mut req = [0u8; 64];
    req[..2].copy_from_slice(&[0x01, 0xB0]);
    let sent = devs
        .iter()
        .any(|d| d.coll.page >= 0xFF00 && d.coll.out_len > 0 && d.write(&req));
    if !sent {
        return;
    }
    let mut s = State::default();
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end
        && (s.level.is_none() || s.power.is_none())
        && s.power != Some(HEADSET_OFF)
    {
        for d in &devs {
            if let Some(r) = d.read(Duration::from_millis(30))
                && r.len() >= 5
            {
                absorb(&mut s, &r);
            }
        }
    }
    let Some(level) = s.level.filter(|_| s.power != Some(HEADSET_OFF)) else {
        return;
    };
    let b = Battery {
        key: ctx.stable_key(&format!("steelseries-1038:{pid:04X}")),
        name: "SteelSeries Arctis Nova Elite".into(),
        via: "Base station".into(),
        kind: Kind::Headset,
        percent: Some(level),
        charging: s.charging || s.power == Some(ON_CABLE),
        ..Battery::default()
    };
    ctx.add(b, VID, pid);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_reply() {
        let mut r = [0u8; 64];
        r[..2].copy_from_slice(&[0x01, 0xB0]);
        r[6] = 64;
        r[14] = 0x02;
        r[15] = 0x02;
        let mut s = State::default();
        absorb(&mut s, &r);
        assert_eq!(
            s,
            State {
                level: Some(64),
                power: Some(0x02),
                charging: true
            }
        );
    }

    #[test]
    fn sage_and_gold_reply() {
        let mut r = [0u8; 64];
        r[..17].copy_from_slice(&[
            0x01, 0xB0, 0x00, 0x00, 0x01, 0x00, 0x63, 0x1A, 0x08, 0x01, 0x02, 0x0A, 0x03, 0x00,
            0x08, 0x08, 0x03,
        ]);
        let mut s = State::default();
        absorb(&mut s, &r);
        assert_eq!((s.level, s.charging), (Some(99), false));
        absorb(&mut s, &[0x07, 0xB7, 0x5F, 0x5A, 0x08]);
        assert_eq!((s.level, s.charging), (Some(95), false));
        assert!(PIDS.contains(&0x2270));
    }

    #[test]
    fn pushed_frames() {
        let mut s = State::default();
        absorb(&mut s, &[0x07, 0xB7, 0x37, 0x00, 0x00, 0x00]);
        absorb(&mut s, &[0x07, 0xB5, 0x00, 0x00, 0x01, 0x00]);
        assert_eq!(
            s,
            State {
                level: Some(55),
                power: Some(HEADSET_OFF),
                charging: false
            }
        );
        let mut s = State::default();
        absorb(&mut s, &[0x07, 0xB7, 0xC8, 0x00, 0x02]);
        assert_eq!(s.level, None);
    }
}
