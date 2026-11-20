use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x24AE;
const VT3_PRO_MAX_RX: u16 = 0x1244;

/// (cable pid, receiver pid, name)
const MODELS: [(u16, u16, &str); 17] = [
    (0x4610, 0x1410, "Rapoo VT0"),
    (0x4620, 0x1420, "Rapoo VT0 Max"),
    (0x4615, 0x1415, "Rapoo VT1"),
    (0x4644, 0x1444, "Rapoo VT1 Max"),
    (0x4622, 0x1422, "Rapoo VT2"),
    (0x4623, 0x1423, "Rapoo VT2 Max"),
    (0x4612, 0x1412, "Rapoo VT3"),
    (0x4617, 0x1417, "Rapoo VT3 Max (Gen 2)"),
    (0x4B06, 0x1B06, "Rapoo VT3 Max V2"),
    (0x4611, 0x1411, "Rapoo VT3S"),
    (0x4621, 0x1421, "Rapoo VT3S Max"),
    (0x4613, 0x1413, "Rapoo VT7"),
    (0x4616, 0x1416, "Rapoo VT7 Max"),
    (0x4614, 0x1414, "Rapoo VT9 (Gen 2)"),
    (0x4618, 0x1418, "Rapoo VT9 Max"),
    (0x4415, 0x1215, "Rapoo VT3 Pro"),
    (0x4433, 0x1233, "Rapoo VT1 Pro Max"),
];

/// Products known to push the short `BB B0` status report.
const SHORT_REPORT: [u16; 5] = [0x1215, 0x4415, 0x1233, 0x4433, VT3_PRO_MAX_RX];

/// How long to wait for a push when none is queued; the next poll drains whatever arrives later.
const WAIT: Duration = Duration::from_secs(1);

fn model(pid: u16) -> Option<(u16, u16, &'static str)> {
    MODELS.iter().copied().find(|m| m.0 == pid || m.1 == pid)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Link {
    Unknown,
    Receiver,
    Bluetooth,
    Cable,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Status {
    percent: Option<u8>,
    charging: bool,
    link: Link,
    mouse_pid: Option<u16>,
}

/// One pushed report including its id. `short` selects the `BB B0` layout.
fn decode(r: &[u8], short: bool) -> Option<Status> {
    let (state, pct, link, mouse_pid) = match r {
        [0x07, mode, _, _, _, _, _, state, pct, rest @ ..] if !short => {
            let link = match mode & 0x0F {
                0 => Link::Receiver,
                1 => Link::Bluetooth,
                2 => Link::Cable,
                _ => return None,
            };
            let pid = (rest.len() >= 6).then(|| u16::from_le_bytes([rest[4], rest[5]]));
            (*state, *pct, link, pid)
        }
        [0xBB, 0xB0, _, _, _, state, pct, ..] if short => (*state, *pct, Link::Unknown, None),
        _ => return None,
    };
    // State 0 is a transition; some short-report firmware sends 7F instead of a level while charging.
    if !matches!(state, 1 | 2) || (pct > 100 && !(short && state == 2 && pct == 0x7F)) {
        return None;
    }
    Some(Status {
        percent: (pct <= 100).then_some(pct),
        charging: state == 2,
        link,
        mouse_pid,
    })
}

/// Handles stay open between polls so Windows queues the pushes that arrive in between.
static OPEN: Mutex<BTreeMap<String, Dev>> = Mutex::new(BTreeMap::new());

fn drain(dev: &Dev, short: bool) -> Option<Status> {
    let mut got = None;
    for _ in 0..64 {
        let Some(r) = dev.read(Duration::from_millis(30)) else {
            break;
        };
        got = decode(&r, short).or(got);
    }
    got
}

fn read_status(open: &mut BTreeMap<String, Dev>, c: &Coll, short: bool) -> Option<Status> {
    let fresh = !open.contains_key(&c.path);
    if fresh {
        open.insert(c.path.clone(), Dev::open(c)?);
    }
    let dev = open.get(&c.path)?;
    if !fresh && let Some(s) = drain(dev, short) {
        return Some(s);
    }
    let started = Instant::now();
    if let Some(s) = dev.read_until(WAIT, |r| decode(r, short)) {
        return Some(s);
    }
    // A kept handle that fails at once most likely lost its receiver: reopen once.
    if !fresh && started.elapsed() < Duration::from_millis(50) {
        let dev = Dev::open(c)?;
        let s = dev.read_until(WAIT.saturating_sub(started.elapsed()), |r| decode(r, short));
        open.insert(c.path.clone(), dev);
        return s;
    }
    None
}

fn wanted(c: &Coll, pid: u16) -> bool {
    c.page == 0xFF00
        && c.usage == 0x0002
        && match c.in_len {
            16 | 19 => true,
            13 => pid == 0x1417,
            7 => SHORT_REPORT.contains(&pid),
            _ => false,
        }
}

fn via(link: Link, pid: u16, model: Option<(u16, u16, &str)>) -> &'static str {
    match link {
        Link::Cable => "USB cable",
        Link::Unknown if model.is_some_and(|m| m.0 == pid) => "USB cable",
        Link::Bluetooth => "Bluetooth",
        _ => "2.4 GHz receiver",
    }
}

pub fn poll_rapoo(ctx: &mut Ctx) {
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let mut present = Vec::new();
    for (_, pid) in ctx.products(VID) {
        for c in ctx.find(VID, pid, |c| wanted(c, pid)) {
            present.push(c.path.clone());
            let short = c.in_len == 7;
            let Some(s) = read_status(&mut open, &c, short) else {
                continue;
            };
            let seen_pid = s.mouse_pid.unwrap_or(pid);
            let m = model(seen_pid);
            let identity = m.map_or(seen_pid, |m| m.0);
            let name = match m.map(|m| m.2) {
                _ if pid == VT3_PRO_MAX_RX => "Rapoo VT3 Pro Max".to_string(),
                Some(n) => n.into(),
                None if c.product.is_empty() || c.product == "Rapoo Gaming Device" => {
                    "Rapoo wireless mouse".into()
                }
                None => c.product.clone(),
            };
            let base = format!("rapoo-{}", hex4(identity));
            let b = Battery {
                key: ctx.stable_key(&base),
                name,
                via: via(s.link, pid, m).into(),
                kind: Kind::Mouse,
                percent: s.percent,
                charging: s.charging,
                grace_secs: 1800,
                ..Default::default()
            };
            // A charging mouse can show on both cable and receiver: keep one entry, the cable's state first.
            if let Some(old) = ctx.out.iter_mut().find(|o| o.key == base) {
                if b.via == "USB cable" {
                    let percent = b.percent.or(old.percent);
                    *old = Battery {
                        percent,
                        vid: VID,
                        ..b
                    };
                } else if old.percent.is_none() {
                    old.percent = b.percent;
                }
                ctx.claim(VID, pid);
            } else {
                ctx.add(b, VID, pid);
            }
            break;
        }
    }
    open.retain(|path, _| present.contains(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_report_with_mouse_pid() {
        let r = [0x07, 0x02, 0, 0, 0, 0, 0, 2, 88, 0, 0, 0, 0, 0x17, 0x46, 0];
        let s = decode(&r, false).unwrap();
        assert_eq!(
            s,
            Status {
                percent: Some(88),
                charging: true,
                link: Link::Cable,
                mouse_pid: Some(0x4617)
            }
        );
    }

    #[test]
    fn long_report_without_pid() {
        let r = [0x07, 0x00, 0, 0, 0, 0, 0, 1, 40, 0, 0, 0, 0];
        let s = decode(&r, false).unwrap();
        assert_eq!(
            (s.percent, s.link, s.mouse_pid),
            (Some(40), Link::Receiver, None)
        );
        assert_eq!(decode(&[0x07, 0x03, 0, 0, 0, 0, 0, 1, 40], false), None);
        assert_eq!(decode(&[0x07, 0x00, 0, 0, 0, 0, 0, 0, 40], false), None);
    }

    #[test]
    fn short_reports() {
        assert_eq!(
            decode(&[0xBB, 0xB0, 0x71, 0x84, 0x03, 0x01, 0x64], true)
                .unwrap()
                .percent,
            Some(100)
        );
        let s = decode(&[0xBB, 0xB0, 0x71, 0x84, 0x03, 0x02, 0x5F], true).unwrap();
        assert_eq!((s.percent, s.charging), (Some(95), true));
        let s = decode(&[0xBB, 0xB0, 0x71, 0x84, 0x03, 0x02, 0x7F], true).unwrap();
        assert_eq!((s.percent, s.charging), (None, true));
        assert_eq!(
            decode(&[0xBB, 0xB0, 0x71, 0x84, 0x03, 0x01, 0x7F], true),
            None
        );
        assert_eq!(
            decode(&[0xBB, 0xB0, 0x71, 0x84, 0x03, 0x01, 0x64], false),
            None
        );
    }
}
