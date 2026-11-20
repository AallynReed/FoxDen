use std::time::Duration;

use super::keychron::launcher_request;
use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1915;
const RECEIVERS: [u16; 2] = [0x0727, 0x0728];
const WAIT: Duration = Duration::from_millis(500);

/// The device on the USB port: a receiver, or a mouse on its cable.
const LOCAL: u8 = 0x01;
/// The mouse behind a receiver.
const REMOTE: u8 = 0x41;

fn cable_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x0726 => "WAIZOWL OGM Pro",
        0x072C => "WAIZOWL OGM Cloud",
        0x07A0 => "WAIZOWL OGM V2",
        0x07A1 => "WAIZOWL Cloud HAWKEYE",
        0x07A2 => "WAIZOWL OGM V2 GV",
        _ => return None,
    })
}

fn model_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0x07B0 => "WAIZOWL OGM Pro",
        0x07F1 | 0x07F2 => "WAIZOWL OGM Cloud",
        0x07F3 => "WAIZOWL OGM V2",
        0x07F4 => "WAIZOWL Cloud HAWKEYE",
        0x07F5 => "WAIZOWL OGM V2 GV",
        _ => return None,
    })
}

fn is_status(target: u8, r: &[u8]) -> bool {
    r.len() >= 13 && r[0] == target && r[2] == 0x8C
}

/// Receiver answer: `(mouse linked, model code)`.
fn parse_link(r: &[u8]) -> Option<(bool, u16)> {
    is_status(LOCAL, r).then(|| (r[4] & 0x08 != 0, u16::from_le_bytes([r[11], r[12]])))
}

/// Mouse answer: `(percent, charging)`; percent 0 means not known yet.
fn parse_power(target: u8, r: &[u8]) -> Option<(u8, bool)> {
    (is_status(target, r) && (1..=100).contains(&r[6])).then_some((r[6], r[5] == 1))
}

fn query(dev: &Dev, target: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&launcher_request(target, 0x01)) {
        return None;
    }
    dev.read_until(WAIT, |r| is_status(target, r).then(|| r.to_vec()))
}

pub fn poll_waizowl(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        let receiver = RECEIVERS.contains(&pid);
        if !receiver && cable_name(pid).is_none() {
            continue;
        }
        let colls = ctx.find(vid, pid, |c: &Coll| {
            c.iface == 3 && c.page == 0xFF0A && c.usage == 1 && c.in_len >= 65 && c.out_len >= 65
        });
        for c in colls {
            let Some(dev) = Dev::open(&c) else { continue };
            let Some(local) = query(&dev, LOCAL) else {
                continue;
            };
            let mut b = Battery {
                key: ctx.stable_key(&format!("waizowl-1915:{}", hex4(pid))),
                name: cable_name(pid).unwrap_or("WAIZOWL mouse").into(),
                kind: Kind::Mouse,
                grace_secs: 1800,
                ..Default::default()
            };
            let power = if receiver {
                let (linked, model) = parse_link(&local).unwrap_or_default();
                if let Some(n) = model_name(model) {
                    b.name = n.into();
                }
                b.via = "2.4 GHz receiver".into();
                b.connected = linked;
                let reply = linked.then(|| query(&dev, REMOTE)).flatten();
                b.asleep = linked && reply.is_none();
                reply.and_then(|r| parse_power(REMOTE, &r))
            } else {
                b.via = "USB cable".into();
                parse_power(LOCAL, &local)
            };
            if let Some((percent, charging)) = power {
                b.percent = Some(percent);
                b.charging = charging;
            }
            ctx.add(b, vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(head: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..head.len()].copy_from_slice(head);
        r
    }

    #[test]
    fn request_sums_to_a1() {
        let q = launcher_request(REMOTE, 0x01);
        assert_eq!(q[..5], [0x00, 0x41, 0x00, 0x81, 0x01]);
        assert_eq!(q[1..].iter().fold(0u8, |s, &b| s.wrapping_add(b)), 0xA1);
    }

    #[test]
    fn receiver_and_mouse_answers() {
        let rx = frame(&[
            0x01, 0x00, 0x8C, 0x01, 0x0A, 0x15, 0x19, 0x28, 0x07, 0x06, 0x01, 0xF5, 0x07,
        ]);
        let (linked, model) = parse_link(&rx).unwrap();
        assert!(linked);
        assert_eq!(model_name(model), Some("WAIZOWL OGM V2 GV"));
        assert_eq!(parse_power(REMOTE, &rx), None);

        let mouse = frame(&[
            0x41, 0x00, 0x8C, 0x01, 0x00, 0x00, 0x47, 0x00, 0x03, 0x02, 0x06,
        ]);
        assert_eq!(parse_power(REMOTE, &mouse), Some((71, false)));
        assert_eq!(parse_power(LOCAL, &mouse), None);
    }

    #[test]
    fn unknown_level_and_unlinked() {
        let mut m = frame(&[0x41, 0x00, 0x8C, 0x01, 0x00, 0x01, 0x00]);
        assert_eq!(parse_power(REMOTE, &m), None);
        m[6] = 40;
        assert_eq!(parse_power(REMOTE, &m), Some((40, true)));
        m[2] = 0x8A;
        assert_eq!(parse_power(REMOTE, &m), None);
        let rx = frame(&[0x01, 0x00, 0x8C, 0x01, 0x02]);
        assert_eq!(parse_link(&rx), Some((false, 0)));
        assert_eq!(cable_name(0x07A2), Some("WAIZOWL OGM V2 GV"));
        assert_eq!(cable_name(0xAC1C), None);
    }
}
