//! Logitech headsets on page FFA0: `prefix, length, 00, payload`, wireless ones through the receiver's bridge.

use std::sync::Mutex;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x046D;
const SW: u8 = 0x01;
const BRIDGE: [u8; 2] = [0x00, 0x03];
const BATTERY: [u8; 2] = [0x01, 0x04];
const WAIT: Duration = Duration::from_millis(1000);

struct Model {
    pid: u16,
    name: &'static str,
    prefix: &'static [u8],
    bridged: bool,
    key: &'static str,
    /// Battery index for firmware that does not answer the lookup.
    fixed: Option<u8>,
}

const P50: &[u8] = &[0x50, 0x23];

/// Cable modes first so the direct link wins.
const MODELS: &[Model] = &[
    Model {
        pid: 0x0B2E,
        name: "Logitech ASTRO A20 X",
        prefix: P50,
        bridged: false,
        key: "logitech-a20x",
        fixed: None,
    },
    Model {
        pid: 0x0B35,
        name: "Logitech ASTRO A20 X",
        prefix: P50,
        bridged: true,
        key: "logitech-a20x",
        fixed: None,
    },
    Model {
        pid: 0x0B18,
        name: "Logitech G522 LIGHTSPEED",
        prefix: P50,
        bridged: true,
        key: "logitech-046D:0B18",
        fixed: None,
    },
    Model {
        pid: 0x0B21,
        name: "Logitech G325 LIGHTSPEED",
        prefix: P50,
        bridged: true,
        key: "logitech-046D:0B21",
        fixed: Some(0x07),
    },
    Model {
        pid: 0x0AF7,
        name: "Logitech G PRO X 2 LIGHTSPEED",
        prefix: &[0x51],
        bridged: true,
        key: "logitech-046D:0AF7",
        fixed: Some(0x04),
    },
];

/// (pid, bridge index, battery index) found by earlier polls.
static INDEXES: Mutex<Vec<(u16, u8, u8)>> = Mutex::new(Vec::new());

fn frame(prefix: &[u8], payload: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    let h = prefix.len();
    f[..h].copy_from_slice(prefix);
    f[h] = payload.len() as u8 + 1;
    f[h + 2..h + 2 + payload.len()].copy_from_slice(payload);
    f
}

fn direct(prefix: &[u8], idx: u8, func: u8, params: &[u8]) -> [u8; 64] {
    let mut p = vec![idx, func | SW];
    p.extend_from_slice(params);
    frame(prefix, &p)
}

fn bridged(prefix: &[u8], bridge: u8, sub: u8, func: u8, sw: u8, params: &[u8]) -> [u8; 64] {
    let n = params.len() + 3;
    let mut p = vec![
        bridge,
        0x10 | sw,
        (n >> 8) as u8 & 0x0F,
        n as u8,
        0x00,
        sub,
        func | sw,
    ];
    p.extend_from_slice(params);
    frame(prefix, &p)
}

/// The bytes after the length and flags bytes.
fn payload<'a>(prefix: &[u8], r: &'a [u8]) -> Option<&'a [u8]> {
    let h = prefix.len();
    let len = *r.get(h)? as usize;
    (r.starts_with(prefix) && len > 1 && h + 1 + len <= r.len()).then(|| &r[h + 2..h + 1 + len])
}

#[derive(Debug, PartialEq)]
enum Reply {
    Params(Vec<u8>),
    Rejected,
    Off,
}

fn direct_reply(idx: u8, func: u8, p: &[u8]) -> Option<Reply> {
    match p {
        [0xFF, i, ..] if *i == idx => Some(Reply::Rejected),
        [i, f, rest @ ..] if *i == idx && *f == func | SW => Some(Reply::Params(rest.to_vec())),
        _ => None,
    }
}

/// The headset's answer through the bridge; the bridge's ack and connect events are skipped.
fn bridged_reply(bridge: u8, sub: u8, func: u8, sw: u8, p: &[u8]) -> Option<Reply> {
    match p {
        [0xFF, b, ..] if *b == bridge => Some(Reply::Rejected),
        [b, 0x00, _, 0x00, ..] if *b == bridge => Some(Reply::Off),
        [b, 0x10, _, _, 0x00, 0xFF, s, ..] if *b == bridge && *s == sub => Some(Reply::Rejected),
        [b, 0x10, _, _, 0x00, s, f, rest @ ..] if *b == bridge && *s == sub && *f == func | sw => {
            Some(Reply::Params(rest.to_vec()))
        }
        _ => None,
    }
}

/// Bridge status parameters: `03 e8 00 00 01` means no headset is linked.
fn bridge_says_off(params: &[u8]) -> bool {
    params.len() >= 4 && params[2] == 0 && params[3] == 0
}

/// Percent and charging from the battery parameters `level, rounded level, state`.
fn level(params: &[u8]) -> Option<(u8, bool)> {
    let &l = params.first().filter(|&&l| l <= 100)?;
    Some((l, matches!(params.get(2), Some(1 | 2))))
}

fn exchange(
    dev: &Dev,
    m: &Model,
    req: &[u8; 64],
    pick: impl Fn(&[u8]) -> Option<Reply>,
) -> Option<Reply> {
    if !dev.write(req) {
        return None;
    }
    dev.read_until(WAIT, |r| payload(m.prefix, r).and_then(&pick))
}

fn ask_direct(dev: &Dev, m: &Model, idx: u8, func: u8, params: &[u8]) -> Option<Reply> {
    exchange(dev, m, &direct(m.prefix, idx, func, params), |p| {
        direct_reply(idx, func, p)
    })
}

fn ask_bridged(dev: &Dev, m: &Model, bridge: u8, sub: u8, sw: u8, params: &[u8]) -> Option<Reply> {
    exchange(
        dev,
        m,
        &bridged(m.prefix, bridge, sub, 0x00, sw, params),
        |p| bridged_reply(bridge, sub, 0x00, sw, p),
    )
}

fn found_index(r: Option<Reply>) -> Option<u8> {
    match r {
        Some(Reply::Params(p)) => p.first().copied().filter(|&i| i != 0),
        _ => None,
    }
}

enum Status {
    Level(u8, bool),
    Off,
}

fn read_direct(dev: &Dev, m: &Model) -> Option<Status> {
    let idx = found_index(ask_direct(dev, m, 0x00, 0x00, &BATTERY))?;
    match ask_direct(dev, m, idx, 0x00, &[])? {
        Reply::Params(p) => level(&p).map(|(l, c)| Status::Level(l, c)),
        _ => None,
    }
}

fn read_bridged(dev: &Dev, m: &Model) -> Option<Status> {
    let cached = INDEXES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|c| c.0 == m.pid)
        .map(|c| (c.1, c.2));
    let (bridge, idx, sw) = match cached {
        Some((b, i)) => (b, i, SW),
        None => {
            let bridge = found_index(ask_direct(dev, m, 0x00, 0x00, &BRIDGE))?;
            match ask_bridged(dev, m, bridge, 0x00, SW, &BATTERY) {
                Some(Reply::Off) => return Some(Status::Off),
                r => match (found_index(r), m.fixed) {
                    (Some(i), _) => (bridge, i, SW),
                    (None, Some(i)) => (bridge, i, 0x0A),
                    (None, None) => return off_or_silent(dev, m, bridge),
                },
            }
        }
    };
    let status = match ask_bridged(dev, m, bridge, idx, sw, &[]) {
        Some(Reply::Params(p)) => level(&p).map(|(l, c)| Status::Level(l, c)),
        Some(Reply::Off) => Some(Status::Off),
        _ => None,
    };
    let mut cache = INDEXES.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|c| c.0 != m.pid);
    match status {
        Some(Status::Level(..)) if sw == SW => cache.push((m.pid, bridge, idx)),
        None => {
            drop(cache);
            return off_or_silent(dev, m, bridge);
        }
        _ => {}
    }
    status
}

fn off_or_silent(dev: &Dev, m: &Model, bridge: u8) -> Option<Status> {
    match ask_direct(dev, m, bridge, 0x00, &[])? {
        Reply::Params(p) if bridge_says_off(&p) => Some(Status::Off),
        _ => None,
    }
}

pub fn poll_logitech_centurion(ctx: &mut Ctx) {
    for m in MODELS {
        if ctx.is_claimed(VID, m.pid) {
            continue;
        }
        let colls = ctx.find(VID, m.pid, |c| {
            c.page == 0xFFA0 && c.usage == 0x0001 && c.out_len >= 64
        });
        let Some(dev) = colls.first().and_then(Dev::open) else {
            continue;
        };
        dev.flush();
        let status = if m.bridged {
            read_bridged(&dev, m)
        } else {
            read_direct(&dev, m)
        };
        let Some(status) = status else { continue };
        let mut b = Battery {
            key: ctx.stable_key(m.key),
            name: m.name.into(),
            via: if m.bridged {
                "2.4 GHz receiver"
            } else {
                "USB cable"
            }
            .into(),
            kind: Kind::Headset,
            grace_secs: 120,
            ..Battery::default()
        };
        match status {
            Status::Level(l, c) => {
                b.percent = Some(l);
                b.charging = c;
            }
            Status::Off => b.connected = false,
        }
        ctx.add(b, VID, m.pid);
        for o in MODELS.iter().filter(|o| o.key == m.key) {
            ctx.claim(VID, o.pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(f: &[u8; 64], bytes: &[u8]) {
        assert_eq!(&f[..bytes.len()], bytes);
        assert!(f[bytes.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn requests() {
        head(
            &direct(P50, 0x00, 0x00, &BRIDGE),
            &[0x50, 0x23, 0x05, 0x00, 0x00, 0x01, 0x00, 0x03],
        );
        head(
            &bridged(P50, 0x03, 0x00, 0x00, SW, &BATTERY),
            &[
                0x50, 0x23, 0x0A, 0x00, 0x03, 0x11, 0x00, 0x05, 0x00, 0x00, 0x01, 0x01, 0x04,
            ],
        );
        head(
            &bridged(P50, 0x03, 0x06, 0x00, SW, &[]),
            &[
                0x50, 0x23, 0x08, 0x00, 0x03, 0x11, 0x00, 0x03, 0x00, 0x06, 0x01,
            ],
        );
        head(
            &direct(P50, 0x00, 0x00, &BATTERY),
            &[0x50, 0x23, 0x05, 0x00, 0x00, 0x01, 0x01, 0x04],
        );
        head(
            &direct(P50, 0x06, 0x00, &[]),
            &[0x50, 0x23, 0x03, 0x00, 0x06, 0x01],
        );
        head(
            &bridged(&[0x51], 0x03, 0x04, 0x00, 0x0A, &[]),
            &[0x51, 0x08, 0x00, 0x03, 0x1A, 0x00, 0x03, 0x00, 0x04, 0x0A],
        );
        let g325 = bridged(P50, 0x03, 0x07, 0x00, 0x0A, &[]);
        assert_eq!(g325[..2], [0x50, 0x23]);
        head(
            &g325,
            &[
                0x50, 0x23, 0x08, 0x00, 0x03, 0x1A, 0x00, 0x03, 0x00, 0x07, 0x0A,
            ],
        );
    }

    fn reply(prefix: &[u8], p: &[u8]) -> Vec<u8> {
        payload(prefix, &frame(prefix, p)).unwrap().to_vec()
    }

    #[test]
    fn lookups() {
        let p = reply(P50, &[0x00, 0x01, 0x03, 0x00, 0x01]);
        assert_eq!(found_index(direct_reply(0x00, 0x00, &p)), Some(3));
        let p = reply(
            P50,
            &[0x03, 0x10, 0x00, 0x06, 0x00, 0x00, 0x01, 0x06, 0x00, 0x03],
        );
        assert_eq!(found_index(bridged_reply(3, 0x00, 0x00, SW, &p)), Some(6));
        let p = reply(
            P50,
            &[0x03, 0x10, 0x00, 0x06, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
        );
        assert_eq!(found_index(bridged_reply(3, 0x00, 0x00, SW, &p)), None);
        let p = reply(P50, &[0x00, 0x01, 0x06, 0x00, 0x03]);
        assert_eq!(found_index(direct_reply(0x00, 0x00, &p)), Some(6));
    }

    #[test]
    fn battery_replies() {
        let p = reply(
            P50,
            &[0x03, 0x10, 0x00, 0x06, 0x00, 0x06, 0x01, 0x4E, 0x50, 0x00],
        );
        match bridged_reply(3, 6, 0x00, SW, &p) {
            Some(Reply::Params(v)) => assert_eq!(level(&v), Some((78, false))),
            other => panic!("{other:?}"),
        }
        let p = reply(P50, &[0x06, 0x01, 0x46, 0x46, 0x02]);
        match direct_reply(6, 0x00, &p) {
            Some(Reply::Params(v)) => assert_eq!(level(&v), Some((70, true))),
            other => panic!("{other:?}"),
        }
        let g325 = [
            0x50, 0x23, 0x0B, 0x00, 0x03, 0x10, 0x00, 0x06, 0x00, 0x07, 0x0A, 0x5F, 0x5A, 0x02,
        ];
        let p = payload(P50, &g325).unwrap();
        match bridged_reply(3, 7, 0x00, 0x0A, p) {
            Some(Reply::Params(v)) => assert_eq!(level(&v), Some((95, true))),
            other => panic!("{other:?}"),
        }
        assert_eq!(bridged_reply(3, 7, 0x00, SW, p), None);
        let pro = [
            0x51, 0x0B, 0x00, 0x03, 0x10, 0x00, 0x06, 0x00, 0x04, 0x0A, 0x64, 0x64, 0x00,
        ];
        let p = payload(&[0x51], &pro).unwrap();
        match bridged_reply(3, 4, 0x00, 0x0A, p) {
            Some(Reply::Params(v)) => assert_eq!(level(&v), Some((100, false))),
            other => panic!("{other:?}"),
        }
        assert_eq!(level(&[0xC8, 0x00, 0x00]), None);
    }

    #[test]
    fn acks_events_and_errors() {
        let ack = reply(P50, &[0x03, 0x11]);
        assert_eq!(bridged_reply(3, 6, 0x00, SW, &ack), None);
        let off = reply(P50, &[0x03, 0x00, 0x00, 0x00]);
        assert_eq!(bridged_reply(3, 6, 0x00, SW, &off), Some(Reply::Off));
        let on = reply(P50, &[0x03, 0x00, 0x00, 0x01]);
        assert_eq!(bridged_reply(3, 6, 0x00, SW, &on), None);
        let refused = reply(P50, &[0xFF, 0x00, 0x01, 0x05]);
        assert_eq!(direct_reply(0x00, 0x00, &refused), Some(Reply::Rejected));
        assert!(bridge_says_off(&[0x03, 0xE8, 0x00, 0x00, 0x01]));
        assert!(!bridge_says_off(&[0x03, 0xE8, 0x00, 0x01, 0x01]));
        assert_eq!(payload(&[0x51], &frame(P50, &[0x00, 0x01])), None);
    }
}
