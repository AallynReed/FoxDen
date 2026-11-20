use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3710;

/// (pid, on the cable, model name for cables; dongles are shared by several mice).
const PRODUCTS: &[(u16, bool, &str)] = &[
    (0x3402, true, "Pulsar X2 V3"),
    (0x3406, true, "Pulsar X2 V3 eS"),
    (0x5402, false, ""),
    (0x5403, false, ""),
];

const READ_BATTERY: [u8; 3] = [0x08, 0x81, 0x01];

/// Feature report 0: the 64-byte packet `[00][08 81 01][0..][sum le]`, sum of bytes 0..62.
fn request() -> Vec<u8> {
    let mut p = vec![0u8; 64];
    p[1..4].copy_from_slice(&READ_BATTERY);
    let sum: u16 = p[..62].iter().map(|&b| b as u16).sum();
    p[62..].copy_from_slice(&sum.to_le_bytes());
    let mut r = vec![0x00];
    r.extend_from_slice(&p);
    r
}

#[derive(Debug, PartialEq)]
enum Answer {
    Level(u8),
    NoReading,
    /// Still working on it; `true` when the status hints the mouse is not linked.
    Pending(bool),
}

/// Packet `[status][08 81 01][..][..][percent]`; status 01 = done.
fn parse_packet(p: &[u8]) -> Option<Answer> {
    if p.len() < 7 || p[1..4] != READ_BATTERY {
        return None;
    }
    Some(match (p[0], p[6]) {
        (0x01, pct) if pct <= 100 => Answer::Level(pct),
        (0x01, _) => Answer::NoReading,
        (s, _) => Answer::Pending(s == 0x05),
    })
}

enum Outcome {
    Level(u8),
    Offline,
}

fn read_battery(dev: &Dev) -> Option<Outcome> {
    let req = request();
    let mut offline = false;
    for _ in 0..3 {
        if !dev.set_feature(&req) {
            return None;
        }
        for _ in 0..6 {
            sleep(Duration::from_millis(50));
            let r = dev.get_feature(0x00)?;
            if r.len() < 65 {
                return None;
            }
            match parse_packet(&r[1..]) {
                Some(Answer::Level(p)) => return Some(Outcome::Level(p)),
                Some(Answer::NoReading) => return None,
                Some(Answer::Pending(off)) => offline = off,
                None => {}
            }
        }
    }
    offline.then_some(Outcome::Offline)
}

pub fn poll_pulsar(ctx: &mut Ctx) {
    for &(pid, cable, model) in PRODUCTS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let mut colls = ctx.find(VID, pid, |c| c.feat_len >= 65);
        // The lower interface rejects the request.
        colls.sort_by_key(|c| std::cmp::Reverse(c.iface));
        for c in colls {
            let Some(outcome) = Dev::open_feature_only(&c).and_then(|d| read_battery(&d)) else {
                continue;
            };
            let level = match outcome {
                Outcome::Level(p) => Some(p),
                Outcome::Offline => None,
            };
            let mut b = Battery {
                key: ctx.stable_key(&format!("pulsar-{VID:04X}:{pid:04X}")),
                kind: Kind::Mouse,
                ..Default::default()
            };
            if cable {
                b.name = model.into();
                b.via = "USB cable".into();
                b.charging = true;
                b.percent = level;
            } else {
                let generic = c.product.is_empty()
                    || c.product.contains("Dongle")
                    || c.product.contains("Receiver");
                b.name = if generic {
                    "Pulsar wireless mouse".into()
                } else {
                    c.product.clone()
                };
                b.via = "2.4 GHz receiver".into();
                // 0% over the dongle means the mouse is off or out of range.
                b.percent = level.filter(|&p| p > 0);
                b.connected = b.percent.is_some();
            }
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_carries_its_sum() {
        let r = request();
        assert_eq!(r.len(), 65);
        assert_eq!(r[1..5], [0x00, 0x08, 0x81, 0x01]);
        assert_eq!(r[63..], [0x8A, 0x00]);
    }

    #[test]
    fn packet_status() {
        assert_eq!(
            parse_packet(&[0x01, 0x08, 0x81, 0x01, 0, 0, 73]),
            Some(Answer::Level(73))
        );
        assert_eq!(
            parse_packet(&[0x01, 0x08, 0x81, 0x01, 0, 0, 0xFF]),
            Some(Answer::NoReading)
        );
        assert_eq!(
            parse_packet(&[0x02, 0x08, 0x81, 0x01, 0, 0, 0]),
            Some(Answer::Pending(false))
        );
        assert_eq!(
            parse_packet(&[0x05, 0x08, 0x81, 0x01, 0, 0, 0]),
            Some(Answer::Pending(true))
        );
        assert_eq!(parse_packet(&[0x01, 0x08, 0x82, 0x01, 0, 0, 73]), None);
    }
}
