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

/// Device-info read: `AA 10 18 [addr LE] 00 [last]`, answered by `55 10 18 [addr LE]` with 24 data bytes at 8.
fn info_request(addr: u16, last: bool) -> Vec<u8> {
    let mut req = vec![0u8; 1 + PAYLOAD];
    req[1..4].copy_from_slice(&[0xAA, 0x10, 0x18]);
    req[4..6].copy_from_slice(&addr.to_le_bytes());
    req[7] = last as u8;
    req
}

fn info_chunk(addr: u16, r: &[u8]) -> Option<Vec<u8>> {
    let a = addr.to_le_bytes();
    (r.len() >= 32 && r[..5] == [0x55, 0x10, 0x18, a[0], a[1]]).then(|| r[8..32].to_vec())
}

/// Work mode at 16, level at 17, charging at 18 of the 48 info bytes.
fn parse_info(data: &[u8]) -> Option<(u8, bool)> {
    let (mode, level, charge) = (*data.get(16)?, *data.get(17)?, *data.get(18)?);
    (mode <= 3 && (1..=100).contains(&level)).then_some((level, charge != 0))
}

fn query_info(dev: &Dev) -> Option<(u8, bool)> {
    let mut data = Vec::with_capacity(48);
    for (addr, last) in [(0u16, false), (24, true)] {
        dev.flush();
        if !dev.write(&info_request(addr, last)) {
            return None;
        }
        data.extend(dev.read_until(Duration::from_millis(500), |r| info_chunk(addr, r))?);
    }
    parse_info(&data)
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
            let level = query(&dev).or_else(|| query(&dev));
            // This dongle can answer 0 here while its device-info block holds the real level.
            let info = (pid == 0xFEFE && matches!(level, None | Some(0)))
                .then(|| query_info(&dev))
                .flatten();
            let (level, charging) = match (info, level) {
                (Some((l, c)), _) => (l, c),
                (None, Some(l)) => (l, false),
                (None, None) => continue,
            };
            let connected = (1..=100).contains(&level);
            let b = Battery {
                key: ctx.stable_key(&format!("sonix-{}:{}", hex4(vid), hex4(pid))),
                name: display_name(&c.product),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Keyboard,
                percent: connected.then_some(level),
                charging: connected && charging,
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
    fn device_info_fallback() {
        let q = info_request(24, true);
        assert_eq!(
            q[..9],
            [0x00, 0xAA, 0x10, 0x18, 0x18, 0x00, 0x00, 0x01, 0x00]
        );
        let rx1 = [
            0x55, 0x10, 0x18, 0, 0, 0, 0, 0, 0x00, 0x00, 0x00, 0x92, 0x45, 0x0C, 0x44, 0x80, 0x49,
            0x01, 0x00, 0x00, 0x66, 0x01, 0x04, 0x10, 0x02, 0x63, 0x00, 0x00, 0x05, 0x00, 0x00,
            0x00,
        ];
        let mut rx2 = [0u8; 32];
        rx2[..7].copy_from_slice(&[0x55, 0x10, 0x18, 0x18, 0x00, 0x00, 0x01]);
        assert_eq!(info_chunk(24, &rx1), None);
        let mut data = info_chunk(0, &rx1).unwrap();
        data.extend(info_chunk(24, &rx2).unwrap());
        assert_eq!(parse_info(&data), Some((99, false)));
        data[17] = 0;
        assert_eq!(parse_info(&data), None);
    }

    #[test]
    fn strips_link_suffix() {
        assert_eq!(display_name("AJAZZ AK820 MAX 2.4G"), "AJAZZ AK820 MAX");
        assert_eq!(display_name(""), "Wireless keyboard");
    }
}
