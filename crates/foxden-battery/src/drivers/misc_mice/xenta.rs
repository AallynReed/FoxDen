//! Xenta receivers (1D57) broadcast `03 model event state level` every ~2 s; nothing is sent to them.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::{mouse, via};
use crate::Ctx;
use crate::hid::Dev;

const VID: u16 = 0x1D57;
const PIDS: [u16; 5] = [0xFA60, 0xFA61, 0xFA65, 0xFA55, 0x2155];
const ARDOR_RECEIVER: u16 = 0xFA60;
const DELUX_RECEIVER: u16 = 0xFA65;

/// Firmware model code, name, level reported in tens (0..10).
const MODELS: &[(u8, &str, bool)] = &[
    (0x73, "Inphic IN10R MAX", false),
    (0x74, "Inphic IN10 MAX", false),
    (0x3D, "Delux M900 Pro", false),
    (0x30, "Delux M900 Pro", false),
    (0x50, "Delux M900 Pro", false),
    (0x55, "Attack Shark X11", false),
    (0x4E, "Attack Shark X3 Max", false),
    (0xE5, "Attack Shark V3 Pro", false),
    (0x24, "Attack Shark V6", false),
    (0x10, "Attack Shark X6 / R1", true),
];

type Frame = [u8; 5];

fn frame(r: &[u8]) -> Option<Frame> {
    let f: Frame = r.get(..5)?.try_into().ok()?;
    (f[0] == 0x03 && matches!(f[2], 0x40 | 0x41)).then_some(f)
}

/// ARDOR GAMING Fury V2 on FA60: model 00, event 40, tier in the high nibble of byte 3.
/// The fifth byte moves with the tier, so it is not checked.
fn ardor_percent(f: &Frame) -> Option<u8> {
    if f[1] != 0x00 || f[2] != 0x40 || f[3] & 0x0F != 0x03 {
        return None;
    }
    match f[3] >> 4 {
        0x1 => Some(25),
        0x3 => Some(50),
        0x7 => Some(75),
        0xF => Some(100),
        _ => None,
    }
}

/// A tier counts once two frames agree.
#[derive(Default)]
struct Ardor {
    candidate: Option<u8>,
    seen: u8,
    confirmed: Option<(u8, Instant)>,
}

impl Ardor {
    fn see(&mut self, pct: u8, at: Instant) {
        if self.candidate == Some(pct) {
            self.seen = (self.seen + 1).min(2);
        } else {
            self.candidate = Some(pct);
            self.seen = 1;
        }
        if self.seen >= 2 {
            self.confirmed = Some((pct, at));
        }
    }

    fn current(&self, now: Instant) -> Option<u8> {
        self.confirmed
            .filter(|&(_, at)| now - at <= Duration::from_secs(45))
            .map(|(p, _)| p)
    }
}

struct Listener {
    dev: Dev,
    last: Option<Frame>,
    prev: Option<Frame>,
    last_at: Instant,
    ardor: Ardor,
}

/// Open handles by collection path, kept between polls so the broadcasts queue up.
static LISTENERS: LazyLock<Mutex<HashMap<String, Listener>>> = LazyLock::new(Default::default);

#[derive(Debug, Default, PartialEq)]
struct Reading {
    name: &'static str,
    percent: Option<u8>,
    charging: bool,
    full: bool,
}

/// What the latest frame says. `agreed`: the two latest frames match (every link-up starts with a false 100%).
fn reading(pid: u16, f: &Frame, agreed: bool, ardor: Option<u8>) -> Reading {
    let cable = matches!(pid, 0xFA55 | 0x2155);
    let (code, state, level) = (f[1], f[3], f[4]);
    let model = MODELS.iter().find(|m| m.0 == code);
    let mut r = Reading {
        name: model.map_or("Wireless mouse", |m| m.1),
        ..Default::default()
    };
    if pid == DELUX_RECEIVER && matches!(code, 0x30 | 0x50) {
        match state {
            0x02 => {
                r.percent = Some(100);
                r.full = true;
            }
            0x03 => {
                r.charging = true;
                r.percent = (level <= 100).then_some(level);
            }
            0x01 if agreed && (1..=100).contains(&level) => r.percent = Some(level),
            _ => {}
        }
    } else if pid == ARDOR_RECEIVER && ardor_percent(f).is_some() {
        r.name = "ARDOR GAMING Fury V2";
        r.percent = ardor;
    } else if cable || matches!(state, 0x02 | 0x03) {
        // The level freezes on external power.
        r.charging = true;
        if state == 0x02 && !cable {
            r.percent = Some(100);
            r.full = true;
        }
    } else if agreed {
        r.percent = match model {
            Some(m) if m.2 => (1..=10).contains(&level).then(|| level * 10),
            _ => (state == 0x01 && (1..=100).contains(&level)).then_some(level),
        };
    }
    r
}

/// Takes the queued frames; a new listener waits up to 5 s for its first two.
fn drain(l: &mut Listener, pid: u16, fresh: bool) {
    let until = Instant::now() + Duration::from_secs(5);
    let mut got = 0;
    loop {
        let wait = if fresh && got < 2 {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            left
        } else {
            Duration::from_millis(30)
        };
        let Some(r) = l.dev.read(wait).filter(|r| r.len() >= 5) else {
            break;
        };
        let Some(f) = frame(&r) else { continue };
        l.prev = l.last.replace(f);
        l.last_at = Instant::now();
        if pid == ARDOR_RECEIVER
            && let Some(p) = ardor_percent(&f)
        {
            l.ardor.see(p, l.last_at);
        }
        got += 1;
    }
}

pub(super) fn poll(ctx: &mut Ctx) {
    let mut listeners = LISTENERS.lock().unwrap_or_else(|e| e.into_inner());
    let mut present = Vec::new();
    for pid in PIDS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for coll in ctx.find(VID, pid, |c| c.page == 0x000A && c.in_len >= 5) {
            present.push(coll.path.clone());
            let fresh = !listeners.contains_key(&coll.path);
            if fresh {
                // Push-only collection: read-only opens even while a vendor app holds it.
                let Some(dev) = Dev::open_read_only(&coll) else {
                    continue;
                };
                let l = Listener {
                    dev,
                    last: None,
                    prev: None,
                    last_at: Instant::now(),
                    ardor: Ardor::default(),
                };
                listeners.insert(coll.path.clone(), l);
            }
            let l = listeners.get_mut(&coll.path).unwrap();
            drain(l, pid, fresh);
            let now = Instant::now();
            let ardor = if pid == ARDOR_RECEIVER {
                l.ardor.current(now)
            } else {
                None
            };
            let Some(last) = l.last else { continue };
            if ardor.is_none() && now - l.last_at > Duration::from_secs(15) {
                continue;
            }
            let r = reading(pid, &last, l.prev == l.last, ardor);
            let cable = matches!(pid, 0xFA55 | 0x2155);
            let mut b = mouse(
                ctx.stable_key(&format!("xenta-{VID:04X}:{pid:04X}")),
                r.name,
                via(cable),
            );
            b.percent = r.percent;
            b.charging = r.charging;
            b.full = r.full;
            b.asleep = r.percent.is_none() && !r.charging;
            ctx.add(b, VID, pid);
            break;
        }
    }
    listeners.retain(|path, _| present.contains(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_with_either_event() {
        assert_eq!(
            frame(&[0x03, 0x24, 0x41, 0x01, 0x64]),
            Some([0x03, 0x24, 0x41, 0x01, 0x64])
        );
        assert_eq!(frame(&[0x03, 0x24, 0x42, 0x01, 0x64]), None);
        assert_eq!(frame(&[0x03, 0x24, 0x41]), None);
    }

    #[test]
    fn ardor_tiers() {
        assert_eq!(ardor_percent(&[0x03, 0x00, 0x40, 0x33, 0x07]), Some(50));
        assert_eq!(ardor_percent(&[0x03, 0x00, 0x40, 0x73, 0x08]), Some(75));
        assert_eq!(ardor_percent(&[0x03, 0x00, 0x40, 0x31, 0x07]), None);
        let t = Instant::now();
        let mut a = Ardor::default();
        a.see(50, t);
        assert_eq!(a.current(t), None);
        a.see(50, t);
        assert_eq!(a.current(t), Some(50));
        a.see(75, t);
        assert_eq!(a.current(t), Some(50));
        assert_eq!(a.current(t + Duration::from_secs(46)), None);
    }

    #[test]
    fn readings() {
        let v6 = [0x03, 0x24, 0x41, 0x01, 0x64];
        assert_eq!(reading(0xFA60, &v6, true, None).percent, Some(100));
        assert_eq!(reading(0xFA60, &v6, false, None).percent, None);
        assert_eq!(reading(0xFA60, &v6, true, None).name, "Attack Shark V6");

        let inphic = [0x03, 0x73, 0x41, 0x01, 0x64];
        assert_eq!(
            reading(0xFA65, &inphic, true, None).name,
            "Inphic IN10R MAX"
        );

        let x6 = [0x03, 0x10, 0x40, 0x01, 0x07];
        assert_eq!(reading(0xFA60, &x6, true, None).percent, Some(70));

        let ardor = [0x03, 0x00, 0x40, 0x33, 0x07];
        let r = reading(0xFA60, &ardor, false, Some(50));
        assert_eq!((r.name, r.percent), ("ARDOR GAMING Fury V2", Some(50)));

        let charging = [0x03, 0x3D, 0x40, 0x03, 0x40];
        assert!(reading(0xFA60, &charging, true, None).charging);
        let delux = [0x03, 0x30, 0x40, 0x03, 0x40];
        assert_eq!(reading(0xFA65, &delux, false, None).percent, Some(0x40));
        assert!(reading(0xFA55, &v6, true, None).charging);
    }
}
