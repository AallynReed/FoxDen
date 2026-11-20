use std::thread::sleep;
use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x25A7;
/// Other receivers in this family use a different level scale.
const PIDS: [u16; 2] = [0xFA07, 0xFA08];
const GET_POWER: u8 = 0x04;
const FRAME: usize = 17;

/// Feature report 08, GetPower; every frame sums to 0x55.
fn request() -> [u8; FRAME] {
    let mut req = [0u8; FRAME];
    req[0] = 0x08;
    req[1] = GET_POWER;
    let sum = req[..16].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    req[16] = 0x55u8.wrapping_sub(sum);
    req
}

/// (percent, charging) from input report 09; the level counts 0..=10.
fn parse(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < FRAME || r.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0x55 {
        return None;
    }
    if r[..3] != [0x09, GET_POWER, 0x00] || r[6] > 10 {
        return None;
    }
    Some((r[6] * 10, r[7] != 0))
}

fn query(cmd: &Dev, reply: &Dev) -> Option<(u8, bool)> {
    let req = request();
    for attempt in 0..2 {
        if attempt > 0 {
            sleep(Duration::from_millis(200));
        }
        // A report queued before the request would pass for the answer.
        reply.flush();
        if !cmd.set_feature(&req) {
            return None;
        }
        if let Some(v) = reply
            .read(Duration::from_millis(1800))
            .as_deref()
            .and_then(parse)
        {
            return Some(v);
        }
    }
    None
}

fn probe(cmd: &Coll, reply: &Coll) -> Option<(u8, bool)> {
    let cmd = Dev::open_feature_only(cmd)?;
    let reply = Dev::open(reply)?;
    query(&cmd, &reply)
}

pub fn poll_areson(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        if !PIDS.contains(&pid) {
            continue;
        }
        let cmds = ctx.find(VID, pid, |c| {
            c.iface == 1 && c.page == 0xFF02 && c.usage == 0x0002 && c.feat_len >= 17
        });
        let replies = ctx.find(VID, pid, |c| {
            c.iface == 1 && c.page == 0xFF01 && c.usage == 0x0000 && c.in_len >= 17
        });
        let found = cmds
            .iter()
            .find_map(|c| replies.iter().find_map(|r| probe(c, r)));
        // Nothing claimed when the mouse is silent, so it keeps its last reading.
        let Some((pct, charging)) = found else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("areson-{}:{}", hex4(VID), hex4(pid))),
            name: "Redragon wireless mouse".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            charging,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_sums_to_0x55() {
        let r = request();
        assert_eq!(&r[..2], &[0x08, 0x04]);
        assert_eq!(r[16], 0x49);
    }

    #[test]
    fn power_reply() {
        let mut r = [0u8; 17];
        r[..8].copy_from_slice(&[0x09, 0x04, 0x00, 0, 0, 0, 8, 1]);
        r[16] = 0x55u8.wrapping_sub(0x09 + 0x04 + 8 + 1);
        assert_eq!(parse(&r), Some((80, true)));
        r[16] ^= 1;
        assert_eq!(parse(&r), None);
    }
}
