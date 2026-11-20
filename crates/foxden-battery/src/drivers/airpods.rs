//! Apple AirPods and Beats: levels from the BLE proximity message they broadcast for iPhones.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::bt::{adverts, bud_level, nodes, split_detail};
use crate::{Battery, Ctx, Kind};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Levels {
    model: u16,
    single: bool,
    left: Option<u8>,
    right: Option<u8>,
    case: Option<u8>,
    left_chg: bool,
    right_chg: bool,
    case_chg: bool,
}

#[derive(Clone, Copy)]
struct Heard {
    levels: Levels,
    rssi: i16,
    at: Instant,
}

static HEARD: Mutex<Option<HashMap<u64, Heard>>> = Mutex::new(None);

/// Headphones with one battery (Max, Beats over-ear and neckbands).
fn single_battery(model: u16) -> bool {
    matches!(
        model,
        0x200A
            | 0x201F
            | 0x202D
            | 0x2003
            | 0x2005
            | 0x2006
            | 0x2009
            | 0x200C
            | 0x200D
            | 0x2010
            | 0x2017
            | 0x2025
    )
}

fn tenth(n: u8) -> Option<u8> {
    (n <= 10).then_some(n * 10)
}

/// The 0x07 proximity TLV (25 bytes, paired mode) among Apple's manufacturer data TLVs.
fn parse(data: &[u8]) -> Option<Levels> {
    let mut rest = data;
    while let [ty, len, tail @ ..] = rest {
        let len = *len as usize;
        let body = tail.get(..len)?;
        if *ty == 0x07
            && let Some(l) = proximity(body)
        {
            return Some(l);
        }
        rest = &tail[len..];
    }
    None
}

fn proximity(b: &[u8]) -> Option<Levels> {
    if b.len() < 25 || b[0] != 0x01 {
        return None;
    }
    let model = u16::from_le_bytes([b[1], b[2]]);
    if model & 0xFF00 != 0x2000 {
        return None;
    }
    let (status, pods, flags) = (b[3], b[4], b[5]);
    let (this, other, chg) = (pods & 0x0F, pods >> 4, flags >> 4);
    let mut l = Levels {
        model,
        single: single_battery(model),
        ..Default::default()
    };
    if l.single {
        let (pct, charging) = match (tenth(this), tenth(other)) {
            (Some(0), Some(o)) if o > 0 => (o, chg & 0x02 != 0),
            (Some(p), _) => (p, chg & 0x01 != 0),
            _ => return None,
        };
        l.left = Some(pct);
        l.left_chg = charging;
        return Some(l);
    }
    let left_sends = status & 0x20 != 0;
    let (lv, rv) = if left_sends {
        (this, other)
    } else {
        (other, this)
    };
    let (lbit, rbit) = if left_sends {
        (0x01, 0x02)
    } else {
        (0x02, 0x01)
    };
    l.left = tenth(lv);
    l.right = tenth(rv);
    l.case = tenth(flags & 0x0F);
    l.left_chg = l.left.is_some() && chg & lbit != 0;
    l.right_chg = l.right.is_some() && chg & rbit != 0;
    l.case_chg = l.case.is_some() && chg & 0x04 != 0;
    (l.left.is_some() || l.right.is_some() || l.case.is_some()).then_some(l)
}

pub(super) fn heard(addr: u64, rssi: i16, data: &[u8]) {
    let Some(levels) = parse(data) else { return };
    let now = Instant::now();
    let mut g = HEARD.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    map.insert(
        addr,
        Heard {
            levels,
            rssi,
            at: now,
        },
    );
    // Addresses rotate.
    map.retain(|_, h| now - h.at < Duration::from_secs(600));
}

struct Paired {
    name: String,
    pid: u16,
    connected: bool,
}

fn paired_apple_audio() -> Vec<Paired> {
    #[derive(Default)]
    struct Acc {
        name: String,
        pid: u16,
        apple: bool,
        connected: bool,
    }
    let mut by_container: HashMap<String, Acc> = HashMap::new();
    for n in nodes::list("BTHENUM") {
        let a = by_container.entry(n.container.clone()).or_default();
        if let Some(v) = n.inst.find("_VID&0001004C") {
            a.apple = true;
            if let Some(p) = n.inst[v..].find("_PID&") {
                let hex: String = n.inst[v + p + 5..]
                    .chars()
                    .take_while(char::is_ascii_hexdigit)
                    .collect();
                a.pid = u16::from_str_radix(&hex, 16).unwrap_or(0);
            }
        }
        if n.inst.starts_with("BTHENUM\\DEV_") {
            a.name = n.name.clone();
            a.connected = n.connected == Some(true);
        }
    }
    let mut out: Vec<Paired> = by_container
        .into_values()
        .filter(|a| {
            let upper = a.name.to_uppercase();
            let looks = upper.contains("AIRPODS") || upper.contains("BEATS");
            (a.apple && a.pid & 0xFF00 == 0x2000) || (a.connected && looks && a.pid == 0)
        })
        .map(|a| Paired {
            name: a.name,
            pid: a.pid,
            connected: a.connected,
        })
        .collect();
    out.sort_by_key(|p| !p.connected);
    out
}

/// The best recent message for a paired device; a missing side is filled from the other bud's message.
fn pick(dev: &Paired) -> Option<Levels> {
    let g = HEARD.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.as_ref()?;
    let now = Instant::now();
    let fresh_for = Duration::from_secs(90);
    let max_age = if dev.connected {
        Duration::from_secs(300)
    } else {
        fresh_for
    };
    let min_rssi = if dev.connected { -90 } else { -80 };
    let mut best: Option<Heard> = None;
    for h in map.values() {
        let age = now - h.at;
        if (dev.pid != 0 && h.levels.model != dev.pid) || age > max_age || h.rssi < min_rssi {
            continue;
        }
        let better = match best {
            None => true,
            Some(b) => {
                let (f, bf) = (age < fresh_for, now - b.at < fresh_for);
                (f && !bf) || (f && bf && h.rssi > b.rssi) || (!f && !bf && h.at > b.at)
            }
        };
        if better {
            best = Some(*h);
        }
    }
    let best = best?;
    let mut l = best.levels;
    if !l.single {
        for h in map.values() {
            let o = h.levels;
            let gap = if h.at > best.at {
                h.at - best.at
            } else {
                best.at - h.at
            };
            if o.model != l.model || o.single || gap > Duration::from_secs(15) {
                continue;
            }
            if l.left.is_none() && o.left.is_some() {
                (l.left, l.left_chg) = (o.left, o.left_chg);
            }
            if l.right.is_none() && o.right.is_some() {
                (l.right, l.right_chg) = (o.right, o.right_chg);
            }
            if l.case.is_none() && o.case.is_some() {
                (l.case, l.case_chg) = (o.case, o.case_chg);
            }
        }
    }
    Some(l)
}

pub fn poll_air_pods(ctx: &mut Ctx) {
    let devices = paired_apple_audio();
    if devices.is_empty() {
        return;
    }
    if adverts::ensure() {
        for _ in 0..15 {
            if devices.iter().any(|d| pick(d).is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let mut used = HashSet::new();
    for d in &devices {
        if d.pid != 0 && !d.connected && used.contains(&d.pid) {
            continue;
        }
        let Some(l) = pick(d) else { continue };
        if d.pid != 0 {
            used.insert(d.pid);
        }
        let name = if d.name.is_empty() {
            "AirPods".to_string()
        } else {
            d.name.clone()
        };
        let mut b = Battery {
            key: ctx.stable_key(&format!("apple-{:04x}", d.pid)),
            name,
            via: "Bluetooth".into(),
            kind: Kind::Headset,
            grace_secs: if d.connected { 300 } else { 0 },
            ..Default::default()
        };
        if l.single {
            b.percent = l.left;
            b.charging = l.left_chg;
        } else {
            let (pct, chg) = bud_level(&[(l.left, l.left_chg), (l.right, l.right_chg)])
                .map_or((l.case, l.case_chg), |(p, c)| (Some(p), c));
            b.percent = pct;
            b.charging = chg;
            b.detail = split_detail(&[
                ("L", l.left, l.left_chg),
                ("R", l.right, l.right_chg),
                ("case", l.case, l.case_chg),
            ]);
        }
        ctx.out.push(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(model: u16, status: u8, pods: u8, flags: u8) -> Vec<u8> {
        let mut f = vec![
            0x07,
            0x19,
            0x01,
            model as u8,
            (model >> 8) as u8,
            status,
            pods,
            flags,
        ];
        f.resize(27, 0);
        f
    }

    #[test]
    fn dual_buds_follow_the_broadcasting_side() {
        // Left broadcasts at 80 %, right 60 %, case 40 % and charging.
        let l = parse(&frame(0x2014, 0x20, 0x68, 0x44)).unwrap();
        assert_eq!((l.left, l.right, l.case), (Some(80), Some(60), Some(40)));
        assert!(l.case_chg && !l.left_chg && !l.right_chg);
        // Right broadcasts: the low nibble is the right bud, bit 0 its charge flag.
        let r = parse(&frame(0x2014, 0x00, 0x6F, 0x1F)).unwrap();
        assert_eq!((r.left, r.right, r.case), (Some(60), None, None));
        assert!(!r.left_chg);
    }

    #[test]
    fn single_battery_models_use_the_low_nibble() {
        let m = parse(&frame(0x200A, 0x00, 0x07, 0x90)).unwrap();
        assert!(m.single);
        assert_eq!(m.left, Some(70));
        assert!(m.left_chg);
    }

    #[test]
    fn other_tlvs_and_pairing_mode_are_skipped() {
        let mut data = vec![0x10, 0x02, 0xAA, 0xBB];
        data.extend(frame(0x2014, 0x20, 0x55, 0x05));
        assert_eq!(parse(&data).map(|l| l.left), Some(Some(50)));
        let mut pairing = frame(0x2014, 0x20, 0x55, 0x05);
        pairing[2] = 0x00;
        assert!(parse(&pairing).is_none());
        assert!(parse(&frame(0x1234, 0x20, 0x55, 0x05)).is_none());
    }
}
