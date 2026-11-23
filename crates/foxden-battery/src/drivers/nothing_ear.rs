//! Nothing and CMF earbuds: the read-only battery request on their RFCOMM control channel.

use std::time::Duration;

use windows_sys::core::GUID;

use super::bt::{nodes, rfcomm::Link, split_detail};
use crate::{Battery, Ctx, Kind};

const SERVICE: GUID = GUID::from_u128(0xaeac4a03_dff5_498f_843a_34487cf133eb);
const BATTERY: u16 = 0xC007;
const BATTERY_PUSH: u16 = 0xE001;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Levels {
    left: Option<(u8, bool)>,
    right: Option<(u8, bool)>,
    case: Option<(u8, bool)>,
}

/// CRC-16/MODBUS.
fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0xFFFFu16, |mut crc, &b| {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
        crc
    })
}

fn request(seq: u8) -> [u8; 10] {
    let [lo, hi] = BATTERY.to_le_bytes();
    let mut f = [0x55, 0x60, 0x01, lo, hi, 0x00, 0x00, seq, 0, 0];
    let crc = crc16(&f[..8]).to_le_bytes();
    f[8..].copy_from_slice(&crc);
    f
}

/// Levels of the last battery frame in the stream.
fn find_status(data: &[u8]) -> Option<Levels> {
    let mut found = None;
    let mut i = 0;
    while i + 10 <= data.len() {
        if data[i..i + 3] != [0x55, 0x60, 0x01] {
            i += 1;
            continue;
        }
        let plen = data[i + 5] as usize;
        let total = 10 + plen;
        let Some(frame) = data.get(i..i + total) else {
            i += 1;
            continue;
        };
        let crc = u16::from_le_bytes([frame[8 + plen], frame[9 + plen]]);
        if crc16(&frame[..8 + plen]) != crc {
            i += 1;
            continue;
        }
        let cmd = u16::from_le_bytes([frame[3], frame[4]]);
        let p = &frame[8..8 + plen];
        if (cmd == BATTERY || cmd == BATTERY_PUSH)
            && let Some((&n, pairs)) = p.split_first()
            && pairs.len() >= n as usize * 2
        {
            let mut l = Levels::default();
            for pair in pairs.as_chunks::<2>().0.iter().take(n as usize) {
                let (pct, chg) = (pair[1] & 0x7F, pair[1] & 0x80 != 0);
                if pct > 100 {
                    continue;
                }
                match pair[0] {
                    0x02 => l.left = Some((pct, chg)),
                    0x03 => l.right = Some((pct, chg)),
                    0x04 => l.case = Some((pct, chg)),
                    _ => {}
                }
            }
            if l != Levels::default() {
                found = Some(l);
            }
        }
        i += total;
    }
    found
}

fn looks_like_nothing(name: &str) -> bool {
    let n = name.to_lowercase();
    (n.contains("nothing") && (n.contains("ear") || n.contains("buds")))
        || n.contains("ear (")
        || n.contains("ear 3a")
        || n.contains("cmf buds")
}

fn read(addr: u64) -> Option<Levels> {
    let link = Link::open(addr, SERVICE, Duration::from_millis(3000))?;
    if !link.send(&request(1)) {
        return None;
    }
    link.read_until(Duration::from_millis(2000), 4096, find_status)
}

pub fn poll_nothing_ear(ctx: &mut Ctx) {
    for (name, addr) in nodes::connected_classic() {
        if !looks_like_nothing(&name) {
            continue;
        }
        let Some(l) = read(addr) else { continue };
        let pct = |p: Option<(u8, bool)>| p.map(|v| v.0);
        let chg = |p: Option<(u8, bool)>| p.is_some_and(|v| v.1);
        let percent = match (pct(l.left), pct(l.right)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut parts = vec![
            ("L", pct(l.left), chg(l.left)),
            ("R", pct(l.right), chg(l.right)),
        ];
        if l.case.is_some() {
            parts.push(("case", pct(l.case), chg(l.case)));
        }
        let b = Battery {
            key: ctx.stable_key(&format!("nothing-{addr:012x}")),
            name,
            via: "Bluetooth".into(),
            kind: Kind::Headset,
            percent,
            charging: chg(l.left) || chg(l.right),
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

    fn reply(cmd: u16, payload: &[u8]) -> Vec<u8> {
        let [lo, hi] = cmd.to_le_bytes();
        let mut f = vec![0x55, 0x60, 0x01, lo, hi, payload.len() as u8, 0x00, 0x01];
        f.extend_from_slice(payload);
        let crc = crc16(&f);
        f.extend_from_slice(&crc.to_le_bytes());
        f
    }

    #[test]
    fn request_frame_is_fixed() {
        let r = request(1);
        assert_eq!(&r[..8], &[0x55, 0x60, 0x01, 0x07, 0xC0, 0x00, 0x00, 0x01]);
        assert_eq!(crc16(b"123456789"), 0x4B37);
    }

    #[test]
    fn battery_reply_lists_each_part() {
        let mut s = vec![0x00, 0x55];
        s.extend(reply(0xC007, &[3, 0x02, 0x50, 0x03, 0xC6, 0x04, 0x28]));
        let l = find_status(&s).unwrap();
        assert_eq!(l.left, Some((80, false)));
        assert_eq!(l.right, Some((70, true)));
        assert_eq!(l.case, Some((40, false)));
    }

    #[test]
    fn other_commands_and_bad_crc_are_ignored() {
        assert_eq!(find_status(&reply(0x4001, &[1, 0x02, 0x50])), None);
        let mut bad = reply(0xE001, &[1, 0x02, 0x50]);
        bad[9] ^= 0xFF;
        assert_eq!(find_status(&bad), None);
    }
}
