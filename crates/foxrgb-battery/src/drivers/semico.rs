use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1A2C;
const DONGLE: u16 = 0x8FFF;
const CABLES: [(u16, &str); 2] = [
    (0xA034, "Attack Shark X98"),
    (0xA036, "AJAZZ AK820 Max Plus"),
];
const MAGIC: [u8; 4] = [0xAA, 0x55, 0xCC, 0x33];
const STATUS: u8 = 0x0E;

fn cable_name(pid: u16) -> Option<&'static str> {
    CABLES.iter().find(|c| c.0 == pid).map(|c| c.1)
}

fn known(pid: u16) -> bool {
    pid == DONGLE || cable_name(pid).is_some()
}

#[derive(Debug, PartialEq)]
enum Status {
    Offline,
    Online(u8),
}

/// `None` for frames that aren't a status reply. State 1..=3 is online; charging isn't reported.
fn parse(r: &[u8]) -> Option<Option<Status>> {
    if r.len() < 7 || r[..4] != MAGIC || r[4] != STATUS {
        return None;
    }
    Some(match (r[5], r[6]) {
        (_, 0) => Some(Status::Offline),
        (pct @ 0..=100, 1..=3) => Some(Status::Online(pct)),
        _ => None,
    })
}

fn query(dev: &Dev) -> Option<Status> {
    let mut req = vec![0u8; 65];
    req[1..5].copy_from_slice(&MAGIC);
    req[5] = STATUS;
    dev.flush();
    if !dev.write(&req) {
        return None;
    }
    dev.read_until(Duration::from_millis(600), parse).flatten()
}

fn keyboard_name(c: &Coll, cable_pid: Option<u16>) -> String {
    if let Some(n) = cable_name(c.pid).or_else(|| cable_pid.and_then(cable_name)) {
        return n.into();
    }
    match c.product.as_str() {
        "" | "2.4 Wireless Device" | "USB Gaming Keyboard" => "Wireless keyboard".into(),
        p => p.into(),
    }
}

pub fn poll_semico(ctx: &mut Ctx) {
    let mut products: Vec<u16> = ctx
        .products(VID)
        .into_iter()
        .map(|p| p.1)
        .filter(|&p| known(p))
        .collect();
    let cable_pid = products.iter().copied().find(|&p| p != DONGLE);
    // The dongle first: a cable plugged in only to charge times out while the keyboard is on 2.4 GHz.
    products.sort_by_key(|&p| p != DONGLE);

    let mut offline = Vec::new();
    for &pid in &products {
        let colls = ctx.find(VID, pid, |c| {
            c.iface == 0
                && c.page == 0xFF02
                && c.usage == 0x0002
                && c.in_len == 65
                && c.out_len == 65
        });
        for c in colls {
            let Some(status) = Dev::open(&c).and_then(|d| query(&d)) else {
                continue;
            };
            let Status::Online(pct) = status else {
                offline.push(pid);
                break;
            };
            let b = Battery {
                key: ctx.stable_key("semico-1A2C"),
                name: keyboard_name(&c, cable_pid),
                via: if pid == DONGLE {
                    "2.4 GHz receiver"
                } else {
                    "USB cable"
                }
                .into(),
                kind: Kind::Keyboard,
                percent: Some(pct),
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, VID, pid);
            for &p in &products {
                ctx.claim(VID, p);
            }
            return;
        }
    }

    if !offline.is_empty() {
        let b = Battery {
            key: ctx.stable_key("semico-1A2C"),
            name: cable_pid
                .and_then(cable_name)
                .unwrap_or("Wireless keyboard")
                .into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Keyboard,
            connected: false,
            vid: VID,
            ..Default::default()
        };
        ctx.out.push(b);
        for pid in offline {
            ctx.claim(VID, pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_replies() {
        assert_eq!(
            parse(&[0xAA, 0x55, 0xCC, 0x33, 0x0E, 96, 1]),
            Some(Some(Status::Online(96)))
        );
        assert_eq!(
            parse(&[0xAA, 0x55, 0xCC, 0x33, 0x0E, 0, 0]),
            Some(Some(Status::Offline))
        );
        assert_eq!(parse(&[0xAA, 0x55, 0xCC, 0x33, 0x0E, 50, 4]), Some(None));
        assert_eq!(parse(&[0xAA, 0x55, 0xCC, 0x33, 0x0F, 50, 1]), None);
    }
}
