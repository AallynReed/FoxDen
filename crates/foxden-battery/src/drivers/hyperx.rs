use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::{clamp100, hex4, interpolate};
use crate::{Battery, Ctx, Kind};

const HP: u16 = 0x03F0;
const KINGSTON: u16 = 0x0951;

#[derive(Clone, Copy)]
enum Proto {
    Cloud3,
    Cloud3S,
    Cloud2,
    Cloud2Kingston,
    Alpha,
    Alpha2,
    Haste,
    Flight,
    Stinger,
}

struct Family {
    vid: u16,
    pids: &'static [u16],
    pick: fn(&Coll) -> bool,
    proto: Proto,
}

fn vendor_page(c: &Coll) -> bool {
    c.page >= 0xFF00
}

const FAMILIES: &[Family] = &[
    Family {
        vid: HP,
        pids: &[0x05B7, 0x0C9D],
        pick: |c| c.out_len == 62 && c.in_len == 62,
        proto: Proto::Cloud3,
    },
    Family {
        vid: HP,
        pids: &[0x069F, 0x0995],
        pick: |c| vendor_page(c) && c.out_len >= 62 && c.in_len >= 62,
        proto: Proto::Cloud3,
    },
    Family {
        vid: HP,
        pids: &[0x06BE, 0x02CC],
        pick: |c| c.out_len == 64 && c.in_len == 64,
        proto: Proto::Cloud3S,
    },
    Family {
        vid: HP,
        pids: &[0x0696, 0x018B, 0x0D93, 0x0C8C],
        pick: |c| c.page == 0xFF90 && c.usage == 0x0303,
        proto: Proto::Cloud2,
    },
    Family {
        vid: KINGSTON,
        pids: &[0x1718],
        pick: |c| c.out_len == 62,
        proto: Proto::Cloud2Kingston,
    },
    Family {
        vid: KINGSTON,
        pids: &[0x16EA, 0x16EB],
        pick: |c| c.page == 0xFF13 && c.usage == 0x0001 && c.in_len == 62 && c.out_len == 62,
        proto: Proto::Cloud2Kingston,
    },
    Family {
        vid: HP,
        pids: &[0x098D],
        pick: |c| vendor_page(c) && c.out_len >= 3,
        proto: Proto::Alpha,
    },
    Family {
        vid: HP,
        pids: &[0x08BE],
        pick: |c| c.page == 0xFF13 && c.usage == 0xFF00,
        proto: Proto::Alpha2,
    },
    Family {
        vid: HP,
        pids: &[0x0AB5, 0x0D97, 0x0F98, 0x0DA0],
        pick: |c| vendor_page(c) && c.out_len >= 2 && c.in_len >= 8,
        proto: Proto::Haste,
    },
    Family {
        vid: KINGSTON,
        pids: &[0x16C4, 0x1723, 0x1749],
        pick: |c| vendor_page(c) && c.out_len >= 3,
        proto: Proto::Flight,
    },
    Family {
        vid: HP,
        pids: &[0x0E90],
        pick: |c| vendor_page(c) && c.out_len >= 3,
        proto: Proto::Flight,
    },
    Family {
        vid: HP,
        pids: &[0x0A93, 0x0A8E],
        pick: stinger_coll,
        proto: Proto::Stinger,
    },
    Family {
        vid: KINGSTON,
        pids: &[0x170B, 0x1713, 0x173F],
        pick: stinger_coll,
        proto: Proto::Stinger,
    },
];

fn stinger_coll(c: &Coll) -> bool {
    c.page == 0xFF00 && c.usage == 0x0001 && c.feat_len >= 32
}

fn name_of(proto: Proto, pid: u16) -> &'static str {
    match (proto, pid) {
        (Proto::Cloud3, 0x069F | 0x0995) => "HyperX Cloud II Core Wireless",
        (Proto::Cloud3, _) => "HyperX Cloud III Wireless",
        (Proto::Cloud3S, _) => "HyperX Cloud III S Wireless",
        (Proto::Cloud2Kingston, 0x16EA | 0x16EB) => "HyperX Cloud Flight S",
        (Proto::Cloud2 | Proto::Cloud2Kingston, _) => "HyperX Cloud II Wireless",
        (Proto::Alpha, _) => "HyperX Cloud Alpha Wireless",
        (Proto::Alpha2, _) => "HyperX Cloud Alpha 2 Wireless",
        (Proto::Haste, 0x0AB5) => "HyperX Pulsefire Haste 2 Core Wireless",
        (Proto::Haste, 0x0DA0) => "HyperX Pulsefire Haste 2 Mini",
        (Proto::Haste, _) => "HyperX Pulsefire Haste 2 Wireless",
        (Proto::Flight, _) => "HyperX Cloud Flight",
        (Proto::Stinger, 0x173F) => "HyperX Cloud Core Wireless",
        (Proto::Stinger, 0x0A8E) => "HyperX Cloud Stinger Core Wireless DTS",
        (Proto::Stinger, _) => "HyperX Cloud Stinger Core Wireless",
    }
}

#[derive(Debug, Default, PartialEq)]
struct Reading {
    connected: bool,
    percent: Option<u8>,
    charging: bool,
    millivolts: u32,
}

impl Reading {
    fn online() -> Self {
        Self {
            connected: true,
            ..Self::default()
        }
    }
}

fn ask(d: &Dev, req: &[u8], ms: u64, want: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    if !d.write(req) {
        return None;
    }
    d.read_until(Duration::from_millis(ms), |r| want(r).then(|| r.to_vec()))
}

fn pct(v: u8) -> Option<u8> {
    (v <= 100).then_some(v)
}

fn be16(hi: u8, lo: u8) -> u32 {
    (hi as u32) << 8 | lo as u32
}

fn cloud3(d: &Dev) -> Option<Reading> {
    let q = |cmd: u8| {
        ask(d, &[0x66, cmd], 800, |r| {
            r.len() >= 5 && r[0] == 0x66 && r[1] == cmd
        })
    };
    let link = q(0x82)?;
    let mut out = Reading {
        connected: link[2] != 0,
        ..Reading::default()
    };
    if out.connected {
        if let Some(r) = q(0x8A) {
            out.charging = r[2] == 1;
        }
        if let Some(r) = q(0x89)
            && r[4] <= 100
        {
            out.percent = Some(r[4]);
            out.millivolts = be16(r[2], r[3]);
        }
    }
    Some(out)
}

fn cloud3s(d: &Dev) -> Option<Reading> {
    let q = |cmd: u8| {
        ask(d, &[0x0C, 0x02, 0x03, 0x01, 0x00, cmd], 1000, |r| {
            r.len() >= 7 && r[0] == 0x0C && r[5] == cmd && r[6] != 0xFF
        })
    };
    let link = q(0x02)?;
    let mut out = Reading {
        connected: link[6] == 2,
        ..Reading::default()
    };
    if out.connected {
        out.percent = q(0x06).and_then(|r| pct(r[6]));
        out.charging = q(0x48).is_some_and(|r| matches!(r[6], 1 | 2));
    }
    Some(out)
}

fn cloud2(d: &Dev) -> Option<Reading> {
    let q = |cmd: u8| {
        ask(d, &[0x06, 0xFF, 0xBB, cmd], 1000, |r| {
            r.len() >= 8 && r[..4] == [0x06, 0xFF, 0xBB, cmd]
        })
    };
    let r = q(0x02)?;
    let percent = pct(r[7])?;
    Some(Reading {
        percent: Some(percent),
        millivolts: be16(r[5], r[6]),
        charging: q(0x03).is_some_and(|r| r[4] == 1),
        ..Reading::online()
    })
}

fn cloud2_kingston(d: &Dev) -> Option<Reading> {
    let q = |cmd: u8| {
        let req = [
            0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00,
            0xBB, cmd, 0x00,
        ];
        ask(d, &req, 1000, |r| {
            r.len() >= 8 && r[0] == 0x0B && r[2] == 0xBB && r[3] == cmd
        })
    };
    let percent = pct(q(0x02)?[7])?;
    Some(Reading {
        percent: Some(percent),
        charging: q(0x03).is_some_and(|r| r[4] == 1),
        ..Reading::online()
    })
}

fn alpha(d: &Dev) -> Option<Reading> {
    let q = |cmd: u8| {
        ask(d, &[0x21, 0xBB, cmd], 1000, |r| {
            r.len() >= 4 && r[0] == 0x21 && r[2] == cmd
        })
    };
    let link = q(0x03)?;
    let mut out = Reading {
        connected: link[3] != 0x01,
        ..Reading::default()
    };
    if out.connected {
        out.charging = q(0x0C).is_some_and(|r| r[3] == 0x01);
        out.percent = q(0x0B).and_then(|r| pct(r[3]));
    }
    Some(out)
}

/// The `52 01` link / `50 02` battery pair shared by Cloud Alpha 2 and the Haste 2 dongles.
fn pair_query(d: &Dev, cmd: u8, sub: u8) -> Option<Vec<u8>> {
    ask(d, &[cmd, sub], 1000, |r| {
        r.len() >= 8 && r[0] == cmd + 1 && r[1] == sub
    })
}

fn parse_pair_battery(r: &[u8]) -> Option<(u8, u8, u32)> {
    (r.len() >= 8 && r[2] <= 100).then(|| (r[2], r[3], r[6] as u32 | (r[7] as u32) << 8))
}

fn alpha2(d: &Dev) -> Option<Reading> {
    let link = pair_query(d, 0x52, 0x01)?;
    let mut out = Reading {
        connected: link[2] != 0,
        ..Reading::default()
    };
    if out.connected
        && let Some((p, state, mv)) = pair_query(d, 0x50, 0x02)
            .as_deref()
            .and_then(parse_pair_battery)
    {
        out.percent = Some(p);
        out.charging = state == 0x01;
        out.millivolts = mv;
    }
    Some(out)
}

const HASTE_SETTLE: Duration = Duration::from_secs(6);

/// Link-up time per dongle; `None` while the mouse is away.
static HASTE_LINK: Mutex<BTreeMap<u16, Option<Instant>>> = Mutex::new(BTreeMap::new());

fn haste(d: &Dev, pid: u16) -> Option<Reading> {
    let link = pair_query(d, 0x52, 0x01);
    let linked = link.as_ref().is_none_or(|r| r[2] != 0);
    let battery = if linked {
        pair_query(d, 0x50, 0x02)
            .as_deref()
            .and_then(parse_pair_battery)
    } else {
        None
    };
    if link.is_none() && battery.is_none() {
        return None;
    }
    let now = Instant::now();
    let settled = {
        let mut map = HASTE_LINK.lock().unwrap_or_else(|e| e.into_inner());
        let first = !map.contains_key(&pid);
        let up = map.entry(pid).or_insert(None);
        if !linked {
            *up = None;
        } else if up.is_none() {
            *up = Some(if first {
                now.checked_sub(HASTE_SETTLE).unwrap_or(now)
            } else {
                now
            });
        }
        up.is_some_and(|t| now.duration_since(t) >= HASTE_SETTLE)
    };
    let mut out = Reading {
        connected: linked,
        ..Reading::default()
    };
    if let Some((p, state, mv)) = battery
        && settled
    {
        out.percent = Some(p);
        out.charging = matches!(state, 0x01 | 0x02);
        out.millivolts = mv;
    }
    Some(out)
}

fn flight_reading(mv: u32) -> Reading {
    let mut out = Reading {
        millivolts: mv,
        ..Reading::online()
    };
    if mv > 0x100B {
        out.charging = true;
    } else if mv <= 3648 {
        out.percent = Some(clamp100((0.00125 * mv as f64) as i32));
    } else if mv > 3975 {
        out.percent = Some(100);
    } else {
        let v = mv as f64;
        let p = 0.000_000_025_475_05 * v.powi(4) - 0.000_390_029_9 * v.powi(3) + 2.238_321 * v * v
            - 5706.256 * v
            + 5_452_299.0;
        out.percent = Some(clamp100(p.round() as i32));
    }
    out
}

fn flight(d: &Dev) -> Option<Reading> {
    let r = ask(d, &[0x21, 0xFF, 0x05], 1000, |r| r.len() >= 5)?;
    Some(flight_reading(be16(r[3], r[4])))
}

const STINGER_LINK: [u8; 20] = [
    0xFF, 0x07, 0x00, 0xFD, 0x04, 0x00, 0x00, 0x02, 0x82, 0x03, 0x05, 0x29, 0x0F, 0x00, 0x00, 0x03,
    0x68, 0x10, 0x00, 0x00,
];
const STINGER_BATTERY: [u8; 12] = [
    0xFF, 0x07, 0x00, 0xFD, 0x04, 0x0C, 0xF1, 0x02, 0x01, 0x04, 0xF0, 0x0C,
];
const STINGER_CHARGING: [(i32, i32); 11] = [
    (4130, 100),
    (4090, 90),
    (4050, 80),
    (4000, 70),
    (3967, 60),
    (3946, 50),
    (3930, 40),
    (3916, 30),
    (3871, 20),
    (3831, 10),
    (3400, 0),
];
const STINGER_DISCHARGING: [(i32, i32); 11] = [
    (4121, 100),
    (4042, 90),
    (3954, 80),
    (3905, 70),
    (3838, 60),
    (3785, 50),
    (3763, 40),
    (3738, 30),
    (3700, 20),
    (3651, 10),
    (3517, 0),
];

fn stinger_fresh(r: &[u8]) -> bool {
    !(r[1] == 0x01 && r[2] == 0x00)
}

fn stinger_is_battery(r: &[u8]) -> bool {
    r[..3] == [0xFF, 0x12, 0x05]
}

/// Battery feature reply: `[9]` state 03 on battery / 05 charging / 06 full, `[11..13]` mV little endian.
fn parse_stinger_battery(r: &[u8]) -> Option<Reading> {
    if r.len() < 13 || !matches!(r[9], 0x03 | 0x05 | 0x06) {
        return None;
    }
    let mv = r[11] as u32 | (r[12] as u32) << 8;
    let charging = r[9] == 0x05;
    let table: &[(i32, i32)] = if charging {
        &STINGER_CHARGING
    } else {
        &STINGER_DISCHARGING
    };
    let percent = if r[9] == 0x06 {
        100
    } else {
        interpolate(mv as i32, table)
    };
    Some(Reading {
        percent: Some(percent),
        charging,
        millivolts: mv,
        ..Reading::online()
    })
}

fn stinger(d: &Dev, pid: u16) -> Option<Reading> {
    let retries = matches!(pid, 0x0A93 | 0x1713 | 0x173F | 0x0A8E);
    let exchange = |cmd: &[u8], good: fn(&[u8]) -> bool| -> (bool, Option<Vec<u8>>) {
        if !d.set_feature(cmd) {
            return (false, None);
        }
        let mut last = None;
        for _ in 0..if retries { 10 } else { 1 } {
            sleep(Duration::from_millis(20));
            let Some(r) = d.get_feature(0xFF).filter(|r| r.len() >= 13) else {
                return (false, last);
            };
            if !retries || good(&r) {
                return (true, Some(r));
            }
            last = Some(r);
        }
        (false, last)
    };
    let link = match exchange(&STINGER_LINK, stinger_fresh) {
        (true, r) => r?,
        (false, Some(r)) if retries && !stinger_fresh(&r) => r,
        _ => return None,
    };
    let mut out = Reading {
        connected: link[11] == 0x01,
        ..Reading::default()
    };
    if out.connected
        && let (true, Some(r)) = exchange(&STINGER_BATTERY, stinger_is_battery)
        && let Some(b) = parse_stinger_battery(&r)
    {
        out = b;
    }
    Some(out)
}

fn read(proto: Proto, d: &Dev, pid: u16) -> Option<Reading> {
    match proto {
        Proto::Cloud3 => cloud3(d),
        Proto::Cloud3S => cloud3s(d),
        Proto::Cloud2 => cloud2(d),
        Proto::Cloud2Kingston => cloud2_kingston(d),
        Proto::Alpha => alpha(d),
        Proto::Alpha2 => alpha2(d),
        Proto::Haste => haste(d, pid),
        Proto::Flight => flight(d),
        Proto::Stinger => stinger(d, pid),
    }
}

pub fn poll_hyper_x(ctx: &mut Ctx) {
    for f in FAMILIES {
        for &pid in f.pids {
            if ctx.is_claimed(f.vid, pid) {
                continue;
            }
            for c in ctx.find(f.vid, pid, f.pick) {
                let Some(r) = Dev::open(&c).and_then(|d| read(f.proto, &d, pid)) else {
                    continue;
                };
                let b = Battery {
                    key: ctx.stable_key(&format!("hyperx-{}:{}", hex4(f.vid), hex4(pid))),
                    name: name_of(f.proto, pid).into(),
                    via: "2.4 GHz receiver".into(),
                    kind: if matches!(f.proto, Proto::Haste) {
                        Kind::Mouse
                    } else {
                        Kind::Headset
                    },
                    percent: r.percent,
                    charging: r.charging,
                    connected: r.connected,
                    ..Battery::default()
                };
                ctx.add(b, f.vid, pid);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_battery_reads_level_state_and_voltage() {
        let r = [0x51, 0x02, 70, 0x01, 0, 0, 0x10, 0x0F];
        assert_eq!(parse_pair_battery(&r), Some((70, 1, 0x0F10)));
        assert_eq!(parse_pair_battery(&[0x51, 0x02, 0xFF, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn cloud_flight_voltage_maps_to_percent() {
        assert!(flight_reading(0x1100).charging);
        assert_eq!(flight_reading(4000).percent, Some(100));
        assert_eq!(flight_reading(3000).percent, Some(3));
        let mid = flight_reading(3800).percent.unwrap();
        assert!((1..100).contains(&mid));
    }

    #[test]
    fn stinger_battery_uses_state_and_curve() {
        let mut r = [0u8; 20];
        r[..3].copy_from_slice(&[0xFF, 0x12, 0x05]);
        r[9] = 0x03;
        r[11..13].copy_from_slice(&3954u16.to_le_bytes());
        let b = parse_stinger_battery(&r).unwrap();
        assert_eq!(
            (b.percent, b.charging, b.millivolts),
            (Some(80), false, 3954)
        );
        r[9] = 0x06;
        assert_eq!(parse_stinger_battery(&r).unwrap().percent, Some(100));
        r[9] = 0x01;
        assert_eq!(parse_stinger_battery(&r), None);
        assert!(stinger_is_battery(&[0xFF, 0x12, 0x05]));
        assert!(!stinger_fresh(&[0xFF, 0x01, 0x00]));
    }
}
