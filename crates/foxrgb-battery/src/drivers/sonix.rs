use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

/// (vid, pid, interface); `None` means any interface with the exact raw collection.
const DONGLES: &[(u16, u16, Option<i32>)] = &[
    (0x05AC, 0x024F, Some(3)),
    (0x0C45, 0xFEFE, Some(3)),
    (0x0C45, 0xFDFD, Some(3)),
    (0x342D, 0xE479, None),
    (0x342D, 0xE485, None),
    (0x342D, 0xE4BC, None),
    (0x342D, 0xE4C2, None),
    (0x342D, 0xE407, None),
    (0x388D, 0x0003, None),
    (0x0DB0, 0x0CA0, None),
    (0x36B0, 0x30BD, None),
    (0x342D, 0xE51A, None),
    (0x342D, 0xE487, None),
    (0x342D, 0xE537, None),
    (0x388D, 0x0002, None),
    (0x19F5, 0x3245, None),
    (0x362D, 0xD028, None),
    (0x362D, 0xD030, None),
    (0x362D, 0xD101, None),
    (0x342D, 0xE49F, None),
];

/// Payload bytes after the report id; the last is a checksum.
const PAYLOAD: usize = 32;

fn request() -> Vec<u8> {
    let mut req = vec![0u8; 1 + PAYLOAD];
    req[1] = 0x20;
    req[2] = 0x01;
    req[PAYLOAD] = req[1..PAYLOAD].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    req
}

/// The level byte of a `20 01 00` reply.
fn parse(r: &[u8]) -> Option<u8> {
    match r {
        [0x20, 0x01, 0x00, level, ..] => Some(*level),
        _ => None,
    }
}

fn query(dev: &Dev) -> Option<u8> {
    dev.flush();
    if !dev.write(&request()) {
        return None;
    }
    dev.read_until(Duration::from_millis(500), parse)
}

fn display_name(product: &str) -> String {
    let n = product
        .strip_suffix(" 2.4G")
        .filter(|s| !s.is_empty())
        .unwrap_or(product);
    if n.is_empty() {
        "Wireless keyboard".into()
    } else {
        n.into()
    }
}

pub fn poll_sonix(ctx: &mut Ctx) {
    for &(vid, pid, iface) in DONGLES {
        if ctx.is_claimed(vid, pid) {
            continue;
        }
        let colls = ctx.find(vid, pid, |c: &Coll| {
            iface.is_none_or(|i| c.iface == i)
                && c.page == 0xFF60
                && c.usage == 0x0061
                && c.out_len as usize == 1 + PAYLOAD
                && c.in_len as usize == 1 + PAYLOAD
        });
        for c in colls {
            let Some(dev) = Dev::open(&c) else { continue };
            // The first request can be missed while the link wakes up.
            let Some(level) = query(&dev).or_else(|| query(&dev)) else {
                continue;
            };
            let connected = (1..=100).contains(&level);
            let b = Battery {
                key: ctx.stable_key(&format!("sonix-{}:{}", hex4(vid), hex4(pid))),
                name: display_name(&c.product),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Keyboard,
                percent: connected.then_some(level),
                connected,
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ends_with_byte_sum() {
        let r = request();
        assert_eq!(r.len(), 33);
        assert_eq!(&r[..3], &[0x00, 0x20, 0x01]);
        assert_eq!(r[32], 0x21);
    }

    #[test]
    fn reply_matches_echo() {
        assert_eq!(parse(&[0x20, 0x01, 0x00, 64, 0, 0]), Some(64));
        assert_eq!(parse(&[0x21, 0x01, 0x00, 64]), None);
        assert_eq!(parse(&[0x20, 0x01]), None);
    }

    #[test]
    fn strips_link_suffix() {
        assert_eq!(display_name("AJAZZ AK820 MAX 2.4G"), "AJAZZ AK820 MAX");
        assert_eq!(display_name(""), "Wireless keyboard");
    }
}
