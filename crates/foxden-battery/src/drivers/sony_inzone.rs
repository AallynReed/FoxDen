use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x054C;
const BATTERY_EVENT: u8 = 0x04;

struct Model {
    pid: u16,
    name: &'static str,
    buds: bool,
}

const MODELS: [Model; 3] = [
    Model {
        pid: 0x0FA8,
        name: "Sony INZONE H9 II",
        buds: false,
    },
    Model {
        pid: 0x0EBF,
        name: "Sony INZONE H5",
        buds: false,
    },
    Model {
        pid: 0x0EC2,
        name: "Sony INZONE Buds",
        buds: true,
    },
];

/// Battery GET to the headset (address 41), tunnelled as vendor command FC00 in report 02.
fn request(tid: u16) -> [u8; 14] {
    let [lo, hi] = tid.to_le_bytes();
    let mut r = [
        0x02,
        12,
        0x01,
        0x00,
        0xFC,
        8,
        0x96,
        0xC3,
        0x41,
        BATTERY_EVENT,
        0x01,
        lo,
        hi,
        0,
    ];
    r[13] = r[6..13].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    r
}

struct Event {
    id: u8,
    kind: u8,
    tid: u16,
    payload: Vec<u8>,
}

/// Event `02 [len] 04 FF .. 00 96 C3 [addr] [id] [type] [tid lo hi] [payload] [sum of 5..=len]`.
fn parse_event(b: &[u8]) -> Option<Event> {
    if b.len() < 15 || b[0] != 0x02 {
        return None;
    }
    let len = b[1] as usize;
    if !(12..=62).contains(&len) || b.len() < len + 2 {
        return None;
    }
    if b[2] != 0x04 || b[3] != 0xFF || b[5] != 0x00 || b[6..8] != [0x96, 0xC3] || b[8] >> 4 != 0x1 {
        return None;
    }
    let sum = b[5..=len].iter().fold(0u8, |s, &x| s.wrapping_add(x));
    if sum != b[len + 1] {
        return None;
    }
    Some(Event {
        id: b[9],
        kind: b[10],
        tid: u16::from_le_bytes([b[11], b[12]]),
        payload: b[13..=len].to_vec(),
    })
}

fn battery_payload(r: &[u8], tid: u16, min: usize) -> Option<Vec<u8>> {
    let e = parse_event(r)?;
    (e.id == BATTERY_EVENT
        && e.tid == tid
        && e.payload.len() >= min
        && matches!(e.kind, 0x10 | 0xA0))
    .then_some(e.payload)
}

static TID: AtomicU16 = AtomicU16::new(1);

/// Transaction ids for this dongle, shared with headset settings so replies never mix.
pub fn next_tid() -> u16 {
    loop {
        let t = TID.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if t > 1 {
            return t;
        }
    }
}

enum Outcome {
    Payload(Vec<u8>),
    Silent,
    Refused,
}

fn ask_battery(d: &Dev, min: usize) -> Outcome {
    let tid = next_tid();
    d.flush();
    if !d.write(&request(tid)) {
        return Outcome::Refused;
    }
    match d.read_until(Duration::from_millis(1500), |r| {
        battery_payload(r, tid, min)
    }) {
        Some(p) => Outcome::Payload(p),
        None => Outcome::Silent,
    }
}

fn valid_pct(v: u8) -> Option<u8> {
    (v <= 100).then_some(v)
}

/// Buds payload `[status][percent]` for left, right and case; FF = bud in the case.
fn parse_buds(p: &[u8]) -> (Option<u8>, String) {
    let (l, r, case) = (valid_pct(p[1]), valid_pct(p[3]), valid_pct(p[5]));
    let percent = match (l, r) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if percent.is_none() {
        return (None, String::new());
    }
    let show = |v: Option<u8>| v.map_or("-".to_string(), |v| format!("{v}%"));
    let mut detail = format!("L {}, R {}", show(l), show(r));
    if case.is_some() {
        detail += &format!(", case {}", show(case));
    }
    (percent, detail)
}

/// Headset payload `[charging][percent]`; percent FF = headset off.
fn parse_headset(p: &[u8]) -> (bool, Option<u8>, bool) {
    (p[1] != 0xFF, valid_pct(p[1]), p[0] != 0)
}

pub fn poll_sony_inzone(ctx: &mut Ctx) {
    for m in &MODELS {
        if ctx.is_claimed(VID, m.pid) {
            continue;
        }
        let min = if m.buds { 6 } else { 2 };
        let mut b = Battery {
            key: ctx.stable_key(&format!("inzone-{}:{}", hex4(VID), hex4(m.pid))),
            name: m.name.into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            ..Battery::default()
        };
        let mut silent = false;
        let mut payload = None;
        for c in ctx.find(VID, m.pid, |c| c.page == 0xFF04 && c.out_len >= 64) {
            let Some(d) = Dev::open(&c) else { continue };
            match ask_battery(&d, min) {
                Outcome::Payload(p) => {
                    payload = Some(p);
                    break;
                }
                Outcome::Silent => silent = true,
                Outcome::Refused => {}
            }
        }
        match payload {
            Some(p) if m.buds => {
                (b.percent, b.detail) = parse_buds(&p);
                b.connected = b.percent.is_some();
            }
            Some(p) => (b.connected, b.percent, b.charging) = parse_headset(&p),
            None if m.buds && silent => b.connected = false,
            None => continue,
        }
        ctx.add(b, VID, m.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(tid: u16, kind: u8, payload: &[u8]) -> Vec<u8> {
        let len = 12 + payload.len();
        let mut b = vec![
            0x02,
            len as u8,
            0x04,
            0xFF,
            0x00,
            0x00,
            0x96,
            0xC3,
            0x14,
            BATTERY_EVENT,
            kind,
        ];
        b.extend_from_slice(&tid.to_le_bytes());
        b.extend_from_slice(payload);
        let sum = b[5..=len].iter().fold(0u8, |s, &x| s.wrapping_add(x));
        b.push(sum);
        b.resize(64, 0);
        b
    }

    #[test]
    fn request_checksum() {
        let r = request(0x0102);
        assert_eq!(r[11..13], [0x02, 0x01]);
        assert_eq!(
            r[13],
            (0x96u32 + 0xC3 + 0x41 + 0x04 + 0x01 + 0x02 + 0x01) as u8
        );
    }

    #[test]
    fn headset_and_buds_replies() {
        let r = event(7, 0x10, &[1, 64]);
        let p = battery_payload(&r, 7, 2).unwrap();
        assert_eq!(parse_headset(&p), (true, Some(64), true));
        assert!(battery_payload(&r, 8, 2).is_none());
        let mut bad = r.clone();
        bad[14] ^= 1;
        assert!(battery_payload(&bad, 7, 2).is_none());
        let buds = event(9, 0xA0, &[0, 80, 0, 0xFF, 0, 55]);
        let p = battery_payload(&buds, 9, 6).unwrap();
        assert_eq!(parse_buds(&p), (Some(80), "L 80%, R -, case 55%".into()));
        assert_eq!(parse_headset(&[0, 0xFF]), (false, None, false));
    }
}
