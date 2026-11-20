use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const LOGITECH: u16 = 0x046D;
const ASTRO: u16 = 0x9886;

const MODELS: &[(u16, &str)] = &[
    (0x002C, "ASTRO A50 Gen 4"),
    (0x002D, "ASTRO A50 Gen 4"),
    (0x002A, "ASTRO A50 Gen 4"),
    (0x0015, "ASTRO A50 Gen 3"),
    (0x0014, "ASTRO A50 Gen 3"),
    (0x0018, "ASTRO A50 Gen 3"),
    (0x001D, "ASTRO A50 Gen 3"),
    (0x0026, "ASTRO A50 Gen 3"),
    (0x0037, "ASTRO A20 Wireless"),
    (0x0038, "ASTRO A20 Wireless"),
    (0x0039, "ASTRO A20 Wireless"),
    (0x0043, "ASTRO A20 Wireless"),
    (0x0044, "ASTRO A20 Wireless"),
    (0x0045, "ASTRO A20 Wireless"),
];

const STATUS: u8 = 0x54;
const BATTERY: u8 = 0x7C;
const UI_STATE: u8 = 0x72;

/// Whether a base station numbers its reports (`02 cmd`) or wants a leading 0 (`00 02 cmd`).
#[derive(Clone, Copy, PartialEq)]
enum Framing {
    Numbered,
    Bare,
}

#[derive(Clone, Copy, Default)]
struct Last {
    percent: u8,
    charging: bool,
    docked: bool,
}

#[derive(Default)]
struct Memory {
    framing: HashMap<u16, Framing>,
    last: HashMap<u16, Last>,
}

static MEMORY: Mutex<Option<Memory>> = Mutex::new(None);

fn remember<T>(f: impl FnOnce(&mut Memory) -> T) -> T {
    let mut g = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Memory::default))
}

/// A50 X and Gen 5 battery status request, no payload.
fn x_request() -> [u8; 64] {
    let mut r = [0u8; 64];
    r[..6].copy_from_slice(&[0x02, 0x0C, 0x03, 0x00, 0x06, 0x0C]);
    r
}

/// Percent and charging from an A50 X / Gen 5 reply; volume and mix events are ignored.
fn x_reply(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 9 || r[0] != 0x02 || r[1] != 0x0C || r[2] < 6 || r[2] as usize + 3 > r.len() {
        return None;
    }
    (r[3] == 0 && r[4] == 0x06 && matches!(r[5], 0x00 | 0x0C) && r[6] <= 100)
        .then(|| (r[6], r[8] != 0))
}

fn legacy_request(f: Framing, cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    if f == Framing::Bare {
        v.push(0);
    }
    v.extend_from_slice(&[0x02, cmd]);
    if !payload.is_empty() {
        v.push(payload.len() as u8);
        v.extend_from_slice(payload);
    }
    v
}

#[derive(Debug, PartialEq)]
enum Answer {
    Ok(Vec<u8>),
    Refused,
    Silent,
}

/// `02 status len payload`: 02 ok, 01 refused, anything else is not ours.
fn legacy_reply(r: &[u8]) -> Option<Answer> {
    match r {
        [0x02, 0x01, _, _, ..] => Some(Answer::Refused),
        [0x02, 0x02, _, _, ..] => Some(Answer::Ok(r.to_vec())),
        _ => None,
    }
}

/// Battery byte (bit 7 charging, low bits percent) from the Gen 3 UI-state read: the field-04 group at data[12..16].
fn gen3_battery(r: &[u8]) -> Option<u8> {
    let data = r.get(3..19)?;
    if (r[2] & 0x3F) < 16 || data[12] != 0x02 || data[13] != 0x04 || data[14] & 0x7F > 100 {
        return None;
    }
    Some(data[14])
}

fn ask_once(dev: &Dev, f: Framing, cmd: u8, payload: &[u8]) -> Option<Answer> {
    dev.flush();
    if !dev.write(&legacy_request(f, cmd, payload)) {
        return None;
    }
    let end = Instant::now() + Duration::from_millis(500);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let r = dev.read(left)?;
        if r.len() < 4 {
            break;
        }
        if let Some(a) = legacy_reply(&r) {
            return Some(a);
        }
    }
    Some(Answer::Silent)
}

/// Tries both framings until the base answers, then sticks to the one that worked.
fn ask(dev: &Dev, pid: u16, cmd: u8, payload: &[u8]) -> Answer {
    let known = remember(|m| m.framing.get(&pid).copied());
    let order = match known {
        Some(Framing::Bare) => [Framing::Bare, Framing::Numbered],
        _ => [Framing::Numbered, Framing::Bare],
    };
    for f in order {
        match ask_once(dev, f, cmd, payload) {
            Some(a @ (Answer::Ok(_) | Answer::Refused)) => {
                remember(|m| m.framing.insert(pid, f));
                return a;
            }
            _ if known.is_some() => break,
            _ => {}
        }
    }
    Answer::Silent
}

fn first_byte(a: &Answer) -> Option<u8> {
    match a {
        Answer::Ok(r) => r.get(3).copied(),
        _ => None,
    }
}

/// Polls one base station. `None` when it gave nothing to report.
fn legacy(c: &Coll, name: &str) -> Option<Battery> {
    let mut b = Battery {
        name: name.into(),
        via: "USB base station".into(),
        kind: Kind::Headset,
        grace_secs: 1800,
        ..Battery::default()
    };
    let Some(dev) = Dev::open(c) else {
        // The vendor app holds the collection exclusively: show the last reading.
        let last = remember(|m| m.last.get(&c.pid).copied());
        if let Some(l) = last {
            b.percent = Some(l.percent);
            b.charging = l.charging;
        }
        let docked = last.is_some_and(|l| l.docked);
        b.detail = if docked {
            "docked, vendor app open"
        } else {
            "vendor app open"
        }
        .into();
        return Some(b);
    };
    let status = first_byte(&ask(&dev, c.pid, STATUS, &[]));
    let docked = status.is_some_and(|s| s & 0x01 != 0);
    if let Some(s) = status {
        b.connected = s & 0x03 != 0;
        if !b.connected {
            return Some(b);
        }
    }
    let mut answer = ask(&dev, c.pid, BATTERY, &[]);
    if answer == Answer::Refused
        && status.is_some()
        && let Answer::Ok(r) = ask(&dev, c.pid, UI_STATE, &[0x01])
        && let Some(v) = gen3_battery(&r)
    {
        answer = Answer::Ok(vec![0x02, 0x02, 0x01, v]);
    }
    match (first_byte(&answer).filter(|v| v & 0x7F <= 100), answer) {
        (Some(v), _) => {
            let pct = v & 0x7F;
            b.connected = true;
            b.percent = Some(pct);
            b.charging = v & 0x80 != 0 || (docked && pct < 100);
            if docked {
                b.detail = "docked".into();
            }
            remember(|m| {
                m.last.insert(
                    c.pid,
                    Last {
                        percent: pct,
                        charging: b.charging,
                        docked,
                    },
                )
            });
            Some(b)
        }
        (None, Answer::Refused) if status.is_none() => {
            b.connected = false;
            Some(b)
        }
        _ => status.map(|_| b),
    }
}

fn legacy_collection(c: &Coll) -> bool {
    let vendor = matches!(c.page, 0xFF31 | 0xFF32 | 0xFF00) && c.in_len >= 64 && c.out_len >= 64;
    let locked = c.page == 0 && matches!(c.iface, 6 | -1);
    vendor || locked
}

fn poll_x(ctx: &mut Ctx) {
    let colls: Vec<Coll> = ctx
        .colls
        .iter()
        .filter(|c| {
            c.vid == LOGITECH
                && matches!(c.pid, 0x0B0B | 0x0B1C)
                && c.page == 0xFF32
                && c.usage == 0x0074
                && c.in_len == 64
                && c.out_len == 64
        })
        .cloned()
        .collect();
    for c in colls {
        if ctx.is_claimed(c.vid, c.pid) {
            continue;
        }
        let Some(dev) = Dev::open(&c) else { continue };
        dev.flush();
        if !dev.write(&x_request()) {
            continue;
        }
        let end = Instant::now() + Duration::from_secs(1);
        let mut found = None;
        for _ in 0..8 {
            let Some(left) = end.checked_duration_since(Instant::now()) else {
                break;
            };
            let Some(r) = dev.read(left) else { break };
            found = x_reply(&r);
            if found.is_some() {
                break;
            }
        }
        let Some((pct, charging)) = found else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("astro-046D:{:04X}", c.pid)),
            name: if c.pid == 0x0B0B {
                "ASTRO A50 X"
            } else {
                "ASTRO A50 Gen 5"
            }
            .into(),
            via: "USB base station".into(),
            kind: Kind::Headset,
            percent: Some(pct),
            charging,
            ..Battery::default()
        };
        ctx.add(b, c.vid, c.pid);
    }
}

pub fn poll_astro(ctx: &mut Ctx) {
    poll_x(ctx);
    for &(pid, name) in MODELS {
        if ctx.is_claimed(ASTRO, pid) {
            continue;
        }
        for c in ctx.find(ASTRO, pid, legacy_collection) {
            if let Some(mut b) = legacy(&c, name) {
                b.key = ctx.stable_key(&format!("astro-9886:{pid:04X}"));
                ctx.add(b, ASTRO, pid);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a50_x_reply() {
        let mut r = [0u8; 64];
        r[..9].copy_from_slice(&[0x02, 0x0C, 0x06, 0x00, 0x06, 0x0C, 0x50, 0x00, 0x01]);
        assert_eq!(x_reply(&r), Some((80, true)));
        r[4] = 0x07;
        assert_eq!(x_reply(&r), None);
        assert_eq!(x_request()[..6], [0x02, 0x0C, 0x03, 0x00, 0x06, 0x0C]);
    }

    #[test]
    fn legacy_framing_and_replies() {
        assert_eq!(
            legacy_request(Framing::Numbered, BATTERY, &[]),
            [0x02, 0x7C]
        );
        assert_eq!(
            legacy_request(Framing::Bare, UI_STATE, &[0x01]),
            [0x00, 0x02, 0x72, 0x01, 0x01]
        );
        assert_eq!(
            legacy_reply(&[0x02, 0x01, 0x00, 0x00]),
            Some(Answer::Refused)
        );
        let ok = [0x02, 0x02, 0x01, 0xB2];
        assert_eq!(first_byte(&legacy_reply(&ok).unwrap()), Some(0xB2));
        assert_eq!(legacy_reply(&[0x03, 0x02, 0x01, 0x10]), None);
    }

    #[test]
    fn gen3_ui_state() {
        let mut r = vec![0x02, 0x02, 0x10];
        r.extend_from_slice(&[
            0x02, 0x01, 0x00, 0x00, 0x02, 0x02, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00, 0x02, 0x04,
            0xC1, 0x00,
        ]);
        assert_eq!(gen3_battery(&r), Some(0xC1));
        r[16] = 0x05;
        assert_eq!(gen3_battery(&r), None);
        assert_eq!(gen3_battery(&[0x02, 0x02, 0x04, 0, 0, 0, 0]), None);
    }
}
