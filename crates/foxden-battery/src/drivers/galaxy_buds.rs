//! Samsung Galaxy Buds: the status frame they push as soon as their SPP channel opens. Nothing is sent.

use std::time::Duration;

use windows_sys::core::GUID;

use super::bt::{nodes, rfcomm::Link, split_detail};
use crate::util::crc16_ccitt;
use crate::{Battery, Ctx, Kind};

/// Buds2 and later, then Buds+ / Live / Pro (standard SPP), then the original Buds.
const SERVICES: [(GUID, bool); 3] = [
    (
        GUID::from_u128(0x2e73a4ad_332d_41fc_90e2_16bef06523f2),
        false,
    ),
    (
        GUID::from_u128(0x00001101_0000_1000_8000_00805f9b34fb),
        false,
    ),
    (
        GUID::from_u128(0x00001102_0000_1000_8000_00805f9b34fd),
        true,
    ),
];

const STATUS: u8 = 0x60;
const EXTENDED_STATUS: u8 = 0x61;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Levels {
    left: Option<u8>,
    right: Option<u8>,
    case: Option<u8>,
}

fn level(v: u8) -> Option<u8> {
    (1..=100).contains(&v).then_some(v)
}

/// Levels of the last valid status frame in the stream. `legacy` is the original Buds' framing.
fn find_status(data: &[u8], legacy: bool) -> Option<Levels> {
    let (start, end) = if legacy { (0xFE, 0xEE) } else { (0xFD, 0xDD) };
    let mut found = None;
    let mut i = 0;
    while i + 6 <= data.len() {
        if data[i] != start {
            i += 1;
            continue;
        }
        let len = if legacy {
            data[i + 2] as usize
        } else {
            let header = u16::from_le_bytes([data[i + 1], data[i + 2]]);
            if header & 0x2000 != 0 {
                i += 1;
                continue;
            }
            (header & 0x3FF) as usize
        };
        if len < 3 || i + 3 + len >= data.len() || data[i + 3 + len] != end {
            i += 1;
            continue;
        }
        let body = &data[i + 3..i + 3 + len];
        let (msg, crc) = body.split_at(len - 2);
        if crc16_ccitt(0, msg) != u16::from_le_bytes([crc[0], crc[1]]) {
            i += 1;
            continue;
        }
        let p = &msg[1..];
        // The case level sits four bytes after the right bud; the original Buds have no case level.
        let at = match msg[0] {
            EXTENDED_STATUS if p.len() >= 4 => Some(2),
            STATUS if p.len() >= 3 => Some(1),
            _ => None,
        };
        if let Some(at) = at {
            let case = if legacy {
                None
            } else {
                p.get(at + 5).copied().and_then(level)
            };
            found = Some(Levels {
                left: level(p[at]),
                right: level(p[at + 1]),
                case,
            });
        }
        i += 4 + len;
    }
    found
}

fn read(addr: u64) -> Option<Levels> {
    SERVICES.iter().find_map(|&(service, legacy)| {
        let link = Link::open(addr, service, Duration::from_millis(4000))?;
        link.read_until(Duration::from_millis(3000), 8192, |buf| {
            find_status(buf, legacy)
        })
    })
}

pub fn poll_galaxy_buds(ctx: &mut Ctx) {
    for (name, addr) in nodes::connected_classic() {
        if !name.to_lowercase().contains("buds") {
            continue;
        }
        let Some(l) = read(addr) else { continue };
        let percent = match (l.left, l.right) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut parts = vec![("L", l.left, false), ("R", l.right, false)];
        if l.case.is_some() {
            parts.push(("case", l.case, false));
        }
        let b = Battery {
            key: ctx.stable_key(&format!("samsung-{addr:012x}")),
            name,
            via: "Bluetooth".into(),
            kind: Kind::Headset,
            percent,
            detail: split_detail(&parts),
            grace_secs: 180,
            ..Default::default()
        };
        ctx.out.push(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(legacy: bool, msg: &[u8]) -> Vec<u8> {
        let len = msg.len() + 2;
        let crc = crc16_ccitt(0, msg);
        let mut f = if legacy {
            vec![0xFE, 0x00, len as u8]
        } else {
            vec![0xFD, len as u8, (len >> 8) as u8]
        };
        f.extend_from_slice(msg);
        f.extend_from_slice(&crc.to_le_bytes());
        f.push(if legacy { 0xEE } else { 0xDD });
        f
    }

    #[test]
    fn extended_status_carries_buds_and_case() {
        let f = frame(false, &[0x61, 0x00, 0x00, 80, 75, 0, 0, 0, 40]);
        assert_eq!(
            find_status(&f, false),
            Some(Levels {
                left: Some(80),
                right: Some(75),
                case: Some(40)
            })
        );
    }

    #[test]
    fn status_update_and_noise_around_it() {
        let mut s = vec![0xFD, 0x01];
        s.extend(frame(false, &[0x60, 0x00, 55, 0, 0, 0, 0, 101]));
        assert_eq!(
            find_status(&s, false),
            Some(Levels {
                left: Some(55),
                right: None,
                case: None
            })
        );
        let mut bad = frame(false, &[0x60, 0x00, 55, 60]);
        bad[5] ^= 1;
        assert_eq!(find_status(&bad, false), None);
    }

    #[test]
    fn original_buds_framing_has_no_case() {
        let f = frame(true, &[0x60, 0x00, 90, 85, 0, 0, 0, 50]);
        assert_eq!(
            find_status(&f, true),
            Some(Levels {
                left: Some(90),
                right: Some(85),
                case: None
            })
        );
        assert_eq!(crc16_ccitt(0, b"123456789"), 0x31C3);
    }
}
