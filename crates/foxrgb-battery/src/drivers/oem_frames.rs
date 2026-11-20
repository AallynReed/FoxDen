//! Request/reply frames that several OEM mouse and keyboard firmwares share.

use std::thread::sleep;
use std::time::Duration;

use crate::hid::{Coll, Dev};

/// What a mouse behind the `0x83` power feature report said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power83 {
    Level { percent: u8, charging: bool },
    Asleep,
}

/// Feature report 0: `[id 0][00][02][02][00][83][00...]`.
pub fn power83_request() -> [u8; 65] {
    let mut r = [0u8; 65];
    r[3] = 0x02;
    r[4] = 0x02;
    r[6] = 0x83;
    r
}

/// The reply with or without its leading report id; `None` for an echo or anything else.
pub fn parse_power83(r: &[u8]) -> Option<Power83> {
    let body = if r.len() >= 9 && r[4] == 0x02 && r[6] == 0x83 {
        &r[1..]
    } else if r.len() >= 8 && r[3] == 0x02 && r[5] == 0x83 {
        r
    } else {
        return None;
    };
    match body[0] {
        0xA0 => Some(Power83::Asleep),
        0xA1 | 0xA2 if body[7] <= 100 => Some(Power83::Level {
            percent: body[7],
            charging: body[6] == 0x01,
        }),
        _ => None,
    }
}

/// Sends the request up to `attempts` times, reading back after each delay in `waits_ms`.
pub fn query_power83(dev: &Dev, attempts: usize, waits_ms: &[u64]) -> Option<Power83> {
    let req = power83_request();
    for _ in 0..attempts {
        if !dev.set_feature(&req) {
            return None;
        }
        for &ms in waits_ms {
            sleep(Duration::from_millis(ms));
            let r = dev.get_feature(0x00)?;
            if let Some(p) = parse_power83(&r) {
                return Some(p);
            }
        }
    }
    None
}

/// Nordic-based status reply to feature report 05, command 15.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status15 {
    pub percent: u8,
    pub charging: bool,
    pub full: bool,
    pub online: bool,
}

pub fn parse_status15(r: &[u8]) -> Option<Status15> {
    if r.len() < 13 || r[0] != 0x05 || r[1] != 0x15 || r[9] > 100 {
        return None;
    }
    Some(Status15 {
        percent: r[9],
        charging: r[10] != 0,
        full: r[11] != 0,
        online: r[12] != 0,
    })
}

/// Up to three tries; stops early once the mouse is reachable (always, on the cable).
pub fn query_status15(dev: &Dev, cable: bool) -> Option<Status15> {
    let mut req = [0u8; 32];
    req[0] = 0x05;
    req[1] = 0x15;
    req[4] = 0x01;
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(Duration::from_millis(90));
        }
        if !dev.set_feature(&req) {
            break;
        }
        // Some receivers need about 90 ms before the answer replaces the echoed request.
        for wait in [15, 75] {
            sleep(Duration::from_millis(wait));
            if let Some(s) = dev.get_feature(0x05).as_deref().and_then(parse_status15) {
                last = Some(s);
                if cable || s.online {
                    return last;
                }
            }
        }
    }
    last
}

/// 64-byte frame on the FF1C:0092 channel: report 04, LE checksum of bytes 3.. at 1..3, type 1A.
pub fn status1a_request() -> [u8; 64] {
    let mut f = [0u8; 64];
    f[0] = 0x04;
    f[3] = 0x1A;
    f[4] = 0x03;
    f[7] = 0x55;
    let sum = f[3..].iter().fold(0u16, |s, &b| s.wrapping_add(b as u16));
    f[1..3].copy_from_slice(&sum.to_le_bytes());
    f
}

/// `(raw level byte, charging)` from a type 1A reply.
pub fn parse_status1a(r: &[u8]) -> Option<(u8, bool)> {
    (r.len() >= 10 && r[0] == 0x04 && r[3] == 0x1A).then(|| (r[8], r[9] != 0))
}

pub fn is_status1a_coll(c: &Coll) -> bool {
    c.page == 0xFF1C && c.usage == 0x0092 && c.in_len == 64 && c.out_len == 64
}

/// Two tries; the receiver only answers while the device is awake.
pub fn query_status1a(c: &Coll, accept: impl Fn(u8) -> bool) -> Option<(u8, bool)> {
    let dev = Dev::open(c)?;
    dev.flush();
    let req = status1a_request();
    for _ in 0..2 {
        if !dev.write_timeout(&req, Duration::from_millis(300)) {
            return None;
        }
        let got = dev.read_until(Duration::from_millis(400), |r| {
            parse_status1a(r).filter(|&(l, _)| accept(l))
        });
        if got.is_some() {
            return got;
        }
    }
    None
}

/// 17-byte `[id][op][00...][sum]` frame whose bytes add up to 0x55.
pub fn frame55(id: u8, op: u8) -> [u8; 17] {
    let mut f = [0u8; 17];
    f[0] = id;
    f[1] = op;
    f[16] = 0x55u8.wrapping_sub(id).wrapping_sub(op);
    f
}

pub fn sum55_ok(r: &[u8]) -> bool {
    r.len() == 17 && r.iter().fold(0u8, |s, &b| s.wrapping_add(b)) == 0x55
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power83_reply_with_and_without_report_id() {
        let with_id = [0x00, 0xA1, 0x00, 0x02, 0x02, 0x00, 0x83, 0x01, 0x4B];
        assert_eq!(
            parse_power83(&with_id),
            Some(Power83::Level {
                percent: 75,
                charging: true
            })
        );
        let bare = [0xA2, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00, 0x20];
        assert_eq!(
            parse_power83(&bare),
            Some(Power83::Level {
                percent: 32,
                charging: false
            })
        );
        let asleep = [0x00, 0xA0, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00, 0x00];
        assert_eq!(parse_power83(&asleep), Some(Power83::Asleep));
        assert_eq!(parse_power83(&power83_request()), None);
    }

    #[test]
    fn status15_fields() {
        let mut r = [0u8; 32];
        r[..2].copy_from_slice(&[0x05, 0x15]);
        r[9] = 64;
        r[10] = 1;
        r[12] = 1;
        assert_eq!(
            parse_status15(&r),
            Some(Status15 {
                percent: 64,
                charging: true,
                full: false,
                online: true
            })
        );
        r[9] = 101;
        assert_eq!(parse_status15(&r), None);
    }

    #[test]
    fn status1a_frame_and_reply() {
        let f = status1a_request();
        assert_eq!(&f[..8], &[0x04, 0x72, 0x00, 0x1A, 0x03, 0x00, 0x00, 0x55]);
        let mut r = [0u8; 64];
        r[0] = 0x04;
        r[3] = 0x1A;
        r[8] = 88;
        r[9] = 1;
        assert_eq!(parse_status1a(&r), Some((88, true)));
    }

    #[test]
    fn frames_sum_to_55() {
        let f = frame55(0x08, 0x04);
        assert_eq!(f[16], 0x49);
        assert!(sum55_ok(&f));
    }
}
