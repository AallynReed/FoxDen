use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3537;
const NOVA_CABLE: u16 = 0x100F;
const CYCLONE2: u16 = 0x100B;

fn is_nova(pid: u16) -> bool {
    matches!(pid, 0x1098 | NOVA_CABLE | 0x1040)
}

/// Nova device-info reply `01 ... [18]`: 0..100 percent, FF while charging (no level then).
fn parse_nova(r: &[u8]) -> Option<Option<u8>> {
    if r.len() < 19 || r[0] != 0x01 {
        return None;
    }
    match r[18] {
        0xFF => Some(None),
        v @ 0..=100 => Some(Some(v)),
        _ => None,
    }
}

/// Cyclone 2 input report 12: `[35]` cable / charging, `[36]` percent.
fn parse_cyclone(r: &[u8]) -> Option<(u8, bool)> {
    (r.len() >= 37 && r[0] == 0x12 && r[36] <= 100).then(|| (r[36], r[35] == 1))
}

fn exchange<T>(
    c: &Coll,
    req: &[u8],
    ms: u64,
    min: usize,
    pick: impl Fn(&[u8]) -> Option<T>,
) -> Option<T> {
    let d = Dev::open(c)?;
    d.flush();
    if !d.write(req) {
        return None;
    }
    let end = Instant::now() + Duration::from_millis(ms);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Some(r) = d.read(left.min(Duration::from_millis(200)))
            && r.len() >= min
            && let Some(v) = pick(&r)
        {
            return Some(v);
        }
    }
    None
}

fn key(pid: u16) -> String {
    format!("gamesir-{}:{}", hex4(VID), hex4(pid))
}

pub fn poll_game_sir(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        if !is_nova(pid) {
            continue;
        }
        for c in ctx.find(VID, pid, |c| c.page == 0xFF7A && c.out_len >= 3) {
            let Some(level) = exchange(&c, &[0x00, 0x01, 0x01], 800, 19, parse_nova) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&key(pid)),
                name: "GameSir Nova".into(),
                via: if pid == NOVA_CABLE {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Gamepad,
                percent: level,
                charging: level.is_none(),
                ..Battery::default()
            };
            ctx.add(b, VID, pid);
            break;
        }
    }
}

pub fn poll_game_sir_cyclone(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, CYCLONE2) {
        return;
    }
    let colls = ctx.find(VID, CYCLONE2, |c| {
        c.page == 0xFFF0 && c.usage == 0x0040 && c.out_len >= 3 && c.in_len >= 37
    });
    for c in colls {
        let Some((percent, charging)) = exchange(&c, &[0x0F, 0x03], 1500, 37, parse_cyclone) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&key(CYCLONE2)),
            name: "GameSir Cyclone 2".into(),
            via: "2.4 GHz receiver or USB".into(),
            kind: Kind::Gamepad,
            percent: Some(percent),
            charging,
            ..Battery::default()
        };
        ctx.add(b, VID, CYCLONE2);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nova_info_reply() {
        let mut r = [0u8; 64];
        r[0] = 0x01;
        r[18] = 73;
        assert_eq!(parse_nova(&r), Some(Some(73)));
        r[18] = 0xFF;
        assert_eq!(parse_nova(&r), Some(None));
        r[18] = 120;
        assert_eq!(parse_nova(&r), None);
        r[0] = 0xF4;
        assert_eq!(parse_nova(&r), None);
    }

    #[test]
    fn cyclone_state_report() {
        let mut r = [0u8; 64];
        r[0] = 0x12;
        r[35] = 1;
        r[36] = 58;
        assert_eq!(parse_cyclone(&r), Some((58, true)));
        r[0] = 0x10;
        assert_eq!(parse_cyclone(&r), None);
    }
}
