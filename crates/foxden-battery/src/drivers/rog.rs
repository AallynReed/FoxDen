use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x0B05;
const OMNI: u16 = 0x1ACE;

struct Model {
    pid: u16,
    name: &'static str,
    keyboard: bool,
    /// 0 when reports are not numbered; 2 / 3 are the Omni receiver channels.
    rid: u8,
    /// -1 = any vendor collection.
    iface: i32,
    col: Option<&'static str>,
    offset: usize,
    scale: u8,
}

const fn kb(pid: u16, name: &'static str, rid: u8, iface: i32, offset: usize) -> Model {
    let col = if rid == 2 { Some("col02") } else { None };
    Model {
        pid,
        name,
        keyboard: true,
        rid,
        iface,
        col,
        offset,
        scale: 1,
    }
}

const fn ms(pid: u16, name: &'static str, rid: u8, iface: i32, offset: usize, scale: u8) -> Model {
    let col = if rid == 3 { Some("col03") } else { None };
    Model {
        pid,
        name,
        keyboard: false,
        rid,
        iface,
        col,
        offset,
        scale,
    }
}

const MODELS: &[Model] = &[
    kb(0x1A85, "ROG Azoth", 2, 2, 6),
    kb(0x1A85, "ROG Azoth", 0, 1, 6),
    kb(0x1A83, "ROG Azoth", 0, 1, 6),
    kb(0x1B42, "ROG Azoth Extreme", 2, 2, 6),
    kb(0x1B3F, "ROG Azoth Extreme", 0, 1, 6),
    kb(0x1CF1, "ROG Azoth Extreme SE", 2, 2, 6),
    kb(0x1CEF, "ROG Azoth Extreme SE", 0, 1, 6),
    kb(0x1C25, "ROG Azoth X", 0, 1, 6),
    kb(0x1C24, "ROG Azoth X", 0, 1, 6),
    kb(0x193E, "ROG Falchion", 0, 1, 11),
    kb(0x193C, "ROG Falchion", 0, 1, 11),
    kb(0x1B04, "ROG Falchion RX", 0, -1, 11),
    kb(0x1B06, "ROG Falchion RX Low Profile", 2, 2, 11),
    kb(0x1AB0, "ROG Strix Scope II 96", 2, 2, 6),
    kb(0x1AAE, "ROG Strix Scope II 96", 0, 1, 6),
    kb(0x1B7A, "ROG Strix Scope II 96 RX", 2, 2, 6),
    kb(0x1B78, "ROG Strix Scope II 96 RX", 0, 1, 6),
    kb(0x1A07, "ROG Strix Scope RX TKL", 0, -1, 6),
    ms(0x18E5, "ROG Chakram", 0, 0, 5, 25),
    ms(0x18E3, "ROG Chakram", 0, 0, 5, 25),
    ms(0x1A1A, "ROG Chakram X", 0, 0, 5, 1),
    ms(0x1A18, "ROG Chakram X", 0, 0, 5, 1),
    ms(0x18A0, "ROG Gladius II Wireless", 0, 2, 5, 25),
    ms(0x197F, "ROG Gladius III Wireless", 0, 0, 5, 1),
    ms(0x197D, "ROG Gladius III Wireless", 0, 0, 5, 1),
    ms(0x1A72, "ROG Gladius III Aimpoint", 3, 2, 5, 1),
    ms(0x1A72, "ROG Gladius III Aimpoint", 0, 0, 5, 1),
    ms(0x1A70, "ROG Gladius III Aimpoint", 0, 0, 5, 1),
    ms(0x1B0C, "ROG Gladius III Aimpoint EVA-02", 0, 0, 5, 1),
    ms(0x1B0A, "ROG Gladius III Aimpoint EVA-02", 0, 0, 5, 1),
    ms(0x1A94, "ROG Harpe Ace Aim Lab Edition", 3, 2, 5, 1),
    ms(0x1A94, "ROG Harpe Ace Aim Lab Edition", 0, 0, 5, 1),
    ms(0x1A92, "ROG Harpe Ace Aim Lab Edition", 0, 0, 5, 1),
    ms(0x1B68, "ROG Harpe Ace Extreme", 3, 2, 5, 1),
    ms(0x1B69, "ROG Harpe Ace Extreme", 3, 2, 5, 1),
    ms(0x1B67, "ROG Harpe Ace Extreme", 0, 0, 5, 1),
    ms(0x1B65, "ROG Harpe Ace Mini", 3, 2, 5, 1),
    ms(0x1B63, "ROG Harpe Ace Mini", 0, 0, 5, 1),
    ms(0x1AD0, "ROG Harpe II Ace", 3, 2, 5, 1),
    ms(0x1C69, "ROG Harpe II Ace", 0, 0, 5, 1),
    ms(0x1960, "ROG Keris Wireless", 0, 0, 5, 25),
    ms(0x195E, "ROG Keris Wireless", 0, 0, 5, 25),
    ms(0x1A59, "ROG Keris Wireless EVA Edition", 0, 0, 5, 25),
    ms(0x1A57, "ROG Keris Wireless EVA Edition", 0, 0, 5, 25),
    ms(0x1A68, "ROG Keris Wireless Aimpoint", 3, 2, 5, 1),
    ms(0x1A6A, "ROG Keris Wireless Aimpoint", 3, 2, 5, 1),
    ms(0x1A68, "ROG Keris Wireless Aimpoint", 0, 0, 5, 1),
    ms(0x1A66, "ROG Keris Wireless Aimpoint", 0, 0, 5, 1),
    ms(0x1B1A, "ROG Keris II Ace", 3, 2, 5, 1),
    ms(0x1B18, "ROG Keris II Ace", 3, 2, 5, 1),
    ms(0x1B16, "ROG Keris II Ace", 0, 0, 5, 1),
    ms(0x1C0E, "ROG Keris II Origin", 3, 2, 5, 1),
    ms(0x1C0C, "ROG Keris II Origin", 0, 0, 5, 1),
    ms(0x1D4E, "ROG Keris II Origin KJP", 3, 2, 5, 1),
    ms(0x1D4C, "ROG Keris II Origin KJP", 0, 0, 5, 1),
    ms(0x1908, "ROG Pugio II", 0, 0, 5, 25),
    ms(0x1906, "ROG Pugio II", 0, 0, 5, 25),
    ms(0x1979, "ROG Spatha X", 0, 0, 5, 1),
    ms(0x1977, "ROG Spatha X", 0, 0, 5, 1),
    ms(0x18B4, "ROG Strix Carry", 0, 1, 7, 25),
    ms(0x1949, "ROG Strix Impact II Wireless", 0, 0, 5, 25),
    ms(0x1947, "ROG Strix Impact II Wireless", 0, 0, 5, 25),
    ms(0x1AD7, "ROG Strix Impact III Wireless", 3, 2, 5, 1),
];

#[derive(Debug, PartialEq)]
enum Reply {
    Ok(Vec<u8>),
    Asleep,
}

fn path_has(c: &Coll, tag: &str) -> bool {
    c.path.to_lowercase().contains(tag)
}

/// `r` with its report id slot in place. `[1..3]` = FF AA: the device behind the receiver did not answer.
fn parse_reply(r: &[u8], cmd: u8, sub: u8) -> Option<Reply> {
    if r.len() < 12 {
        return None;
    }
    if r[1] == 0xFF && r[2] == 0xAA {
        Some(Reply::Asleep)
    } else if r[1] == cmd && r[2] == sub {
        Some(Reply::Ok(r.to_vec()))
    } else {
        None
    }
}

fn query(d: &Dev, rid: u8, cmd: u8, sub: u8) -> Option<Reply> {
    d.flush();
    if !d.write(&[rid, cmd, sub]) {
        return None;
    }
    d.read_until(Duration::from_millis(500), |r| {
        if rid == 0 {
            let mut v = Vec::with_capacity(r.len() + 1);
            v.push(0);
            v.extend_from_slice(r);
            parse_reply(&v, cmd, sub)
        } else {
            parse_reply(r, cmd, sub)
        }
    })
}

fn decode(m: &Model, r: &[u8]) -> (Option<u8>, bool, bool) {
    if m.keyboard {
        let v = r[m.offset];
        ((v <= 100).then_some(v), r[9] == 1, false)
    } else {
        let v = r[m.offset] as u32 * m.scale as u32;
        let ok = (1..=100).contains(&v);
        (ok.then_some(v as u8), r[10] != 0, !ok)
    }
}

fn battery_of(d: &Dev, m: &Model) -> Option<Battery> {
    let reply = query(d, m.rid, 0x12, if m.keyboard { 0x01 } else { 0x07 })?;
    let (percent, charging, asleep) = match reply {
        Reply::Asleep => (None, false, true),
        Reply::Ok(r) => decode(m, &r),
    };
    Some(Battery {
        name: m.name.into(),
        via: if m.rid != 0 {
            "ROG Omni Receiver"
        } else {
            "2.4 GHz receiver / USB"
        }
        .into(),
        kind: if m.keyboard {
            Kind::Keyboard
        } else {
            Kind::Mouse
        },
        percent,
        charging,
        asleep,
        vid: VID,
        ..Battery::default()
    })
}

fn key(pid: u16, part: &str) -> String {
    format!("rog-{}:{}-{part}", hex4(VID), hex4(pid))
}

fn push_unique(ctx: &mut Ctx, mut b: Battery, pid: u16, keyboard: bool) {
    if ctx.out.iter().any(|o| o.vid == VID && o.name == b.name) {
        return;
    }
    b.key = ctx.stable_key(&key(pid, if keyboard { "kb" } else { "ms" }));
    ctx.out.push(b);
}

/// Paired product ids from the receiver's device list, 4-byte slots from byte 5.
fn paired_pids(r: &[u8]) -> Vec<u16> {
    r.get(5..)
        .unwrap_or_default()
        .as_chunks::<4>().0.iter()
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .take_while(|&p| p != 0)
        .collect()
}

fn poll_receiver(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, OMNI) {
        return;
    }
    let omni = |c: &Coll| c.iface == 2 && c.out_len >= 12;
    let Some(list) = ctx
        .find(VID, OMNI, |c| omni(c) && path_has(c, "col01"))
        .into_iter()
        .next()
    else {
        return;
    };
    let Some(Reply::Ok(r)) = Dev::open(&list).and_then(|d| query(&d, 0x01, 0xA0, 0x00)) else {
        return;
    };
    ctx.claim(VID, OMNI);
    for pid in paired_pids(&r) {
        let Some(m) = MODELS.iter().find(|m| m.pid == pid && m.rid != 0) else {
            continue;
        };
        let col = m.col.unwrap_or_default();
        for c in ctx.find(VID, OMNI, |c| omni(c) && path_has(c, col)) {
            if let Some(b) = Dev::open(&c).and_then(|d| battery_of(&d, m)) {
                push_unique(ctx, b, m.pid, m.keyboard);
                break;
            }
        }
    }
}

static AWAKE: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

fn poll_models(ctx: &mut Ctx) {
    for m in MODELS {
        if ctx.is_claimed(VID, m.pid) {
            continue;
        }
        let colls = ctx.find(VID, m.pid, |c| {
            c.out_len >= 12
                && c.page >= 0xFF00
                && (m.iface < 0 || c.iface == m.iface)
                && m.col.is_none_or(|t| path_has(c, t))
        });
        for c in colls {
            let Some(b) = Dev::open(&c).and_then(|d| battery_of(&d, m)) else {
                continue;
            };
            if m.rid != 0 {
                let mut awake = AWAKE.lock().unwrap_or_else(|e| e.into_inner());
                if !b.asleep {
                    awake.insert(m.pid);
                } else if !awake.contains(&m.pid) {
                    continue;
                }
            }
            push_unique(ctx, b, m.pid, m.keyboard);
            ctx.claim(VID, m.pid);
            break;
        }
    }
}

/// Headset dongle reply: `CC FF AA` or `[5..7]` = FF AA while the headset sleeps.
fn parse_headset(r: &[u8], cmd: u8) -> Option<Reply> {
    if r.len() < 8 || r[0] != 0xCC {
        return None;
    }
    if r[1] == 0xFF && r[2] == 0xAA {
        return Some(Reply::Asleep);
    }
    if r[1] != 0x12 {
        return None;
    }
    if r[5] == 0xFF && r[6] == 0xAA {
        Some(Reply::Asleep)
    } else if r[2] == cmd {
        Some(Reply::Ok(r.to_vec()))
    } else {
        None
    }
}

fn headset_query(d: &Dev, cmd: u8) -> Option<Reply> {
    let end = Instant::now() + Duration::from_millis(1200);
    let req = [0xCC, 0x12, cmd];
    d.flush();
    let first = d
        .set_output_and_read(&req, Duration::from_millis(1200))
        .or_else(|| {
            let left = end.checked_duration_since(Instant::now())?;
            if d.write(&req) { d.read(left) } else { None }
        })?;
    if let Some(r) = parse_headset(&first, cmd) {
        return Some(r);
    }
    d.read_until(end.checked_duration_since(Instant::now())?, |r| {
        parse_headset(r, cmd)
    })
}

fn headset_colls(ctx: &Ctx, pid: u16) -> Vec<Coll> {
    ctx.find(VID, pid, |c| {
        c.iface == 3 && c.page == 0xFF00 && c.usage == 0x0001 && c.in_len >= 8 && c.out_len >= 8
    })
}

fn headset(name: &str) -> Battery {
    Battery {
        name: name.into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Headset,
        vid: VID,
        ..Battery::default()
    }
}

fn poll_headsets(ctx: &mut Ctx) {
    for (pid, name) in [(0x1B84, "ROG Pelta"), (0x1AFA, "ROG Delta II")] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for c in headset_colls(ctx, pid) {
            let Some(d) = Dev::open(&c) else { continue };
            let Some(reply) = headset_query(&d, 0x07) else {
                continue;
            };
            let mut b = headset(name);
            match reply {
                Reply::Asleep => b.asleep = true,
                Reply::Ok(r) => {
                    b.percent = (r[6] <= 100).then_some(r[6]);
                    b.asleep = b.percent.is_none();
                    b.charging =
                        matches!(headset_query(&d, 0x08), Some(Reply::Ok(ch)) if ch[5] == 1);
                }
            }
            b.key = ctx.stable_key(&key(pid, "hs"));
            ctx.out.push(b);
            ctx.claim(VID, pid);
            break;
        }
    }
}

/// Earbud reply to `CC 12 12`: `[5]` left, `[6]` right, `[7]` case; 0 = not reported.
fn parse_buds(r: &[u8]) -> (Option<u8>, String) {
    let level = |i: usize| r.get(i).copied().filter(|v| (1..=100).contains(v));
    let (l, rt, case) = (level(5), level(6), level(7));
    let percent = match (l, rt) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if percent.is_none() {
        return (None, String::new());
    }
    let detail = [("L", l), ("R", rt), ("case", case)]
        .iter()
        .filter_map(|(n, v)| v.map(|v| format!("{n} {v}%")))
        .collect::<Vec<_>>()
        .join(", ");
    (percent, detail)
}

fn poll_cetra(ctx: &mut Ctx) {
    const PID: u16 = 0x1CAC;
    if ctx.is_claimed(VID, PID) {
        return;
    }
    for c in headset_colls(ctx, PID) {
        let Some(reply) = Dev::open(&c).and_then(|d| headset_query(&d, 0x12)) else {
            continue;
        };
        let mut b = headset("ROG Cetra Open Wireless");
        if let Reply::Ok(r) = reply {
            (b.percent, b.detail) = parse_buds(&r);
        }
        b.asleep = b.percent.is_none();
        b.key = ctx.stable_key(&key(PID, "hs"));
        ctx.out.push(b);
        ctx.claim(VID, PID);
        return;
    }
}

pub fn poll_rog(ctx: &mut Ctx) {
    poll_headsets(ctx);
    poll_cetra(ctx);
    poll_receiver(ctx);
    poll_models(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_match_command_or_sleep() {
        let mut r = vec![0u8; 64];
        r[1..3].copy_from_slice(&[0x12, 0x07]);
        r[5] = 3;
        r[10] = 1;
        let Some(Reply::Ok(v)) = parse_reply(&r, 0x12, 0x07) else {
            panic!()
        };
        let gladius2 = MODELS.iter().find(|m| m.pid == 0x18A0).unwrap();
        assert_eq!(decode(gladius2, &v), (Some(75), true, false));
        assert_eq!(parse_reply(&r, 0x12, 0x01), None);
        r[1..3].copy_from_slice(&[0xFF, 0xAA]);
        assert_eq!(parse_reply(&r, 0x12, 0x07), Some(Reply::Asleep));
    }

    #[test]
    fn keyboard_offsets_follow_model() {
        let mut r = vec![0u8; 64];
        r[11] = 88;
        r[9] = 1;
        let falchion = MODELS.iter().find(|m| m.pid == 0x193E).unwrap();
        assert_eq!(decode(falchion, &r), (Some(88), true, false));
    }

    #[test]
    fn omni_list_and_headset_replies() {
        let r = [
            0x01, 0xA0, 0, 0, 0, 0x72, 0x1A, 0, 0, 0x85, 0x1A, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(paired_pids(&r), vec![0x1A72, 0x1A85]);
        let mut h = [0u8; 64];
        h[..3].copy_from_slice(&[0xCC, 0x12, 0x07]);
        h[6] = 55;
        assert!(matches!(parse_headset(&h, 0x07), Some(Reply::Ok(_))));
        h[5..7].copy_from_slice(&[0xFF, 0xAA]);
        assert_eq!(parse_headset(&h, 0x07), Some(Reply::Asleep));
    }

    #[test]
    fn buds_take_the_lower_earbud() {
        let mut r = [0u8; 16];
        r[5..8].copy_from_slice(&[99, 80, 86]);
        assert_eq!(parse_buds(&r), (Some(80), "L 99%, R 80%, case 86%".into()));
        r[5..8].copy_from_slice(&[0, 0, 86]);
        assert_eq!(parse_buds(&r), (None, String::new()));
    }
}
