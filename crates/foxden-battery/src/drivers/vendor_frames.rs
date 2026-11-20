//! Frames and tables shared by several vendor drivers.

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use crate::hid::Dev;

/// G-Wolves model names, by product id; the 3854 dongle is shared and stays generic.
pub fn gwolves_name(pid: u16) -> &'static str {
    match pid {
        0x3808 | 0x3817 => "G-Wolves HTM Plus",
        0x6808 | 0x6817 => "G-Wolves HSK Pro 2.0",
        0x5608 | 0x5617 | 0x5618 | 0x5619 => "G-Wolves HTXU",
        0x3608 | 0x3617 | 0x3619 => "G-Wolves Fenrir Pro",
        0x3908 | 0x3917 => "G-Wolves VUK",
        0x7904 | 0x7913 => "G-Wolves HT-S2",
        0x3708 | 0x3717 => "G-Wolves Fenrir Max",
        0x5308 | 0x5317 => "G-Wolves HTS Ultra",
        0x7704 | 0x7713 => "G-Wolves HTR",
        0x3508 | 0x3517 => "G-Wolves Fenrir",
        0x7908 | 0x7917 => "G-Wolves HT-S2 Pro",
        0x5808 | 0x5817 => "G-Wolves HSK Pro",
        0x2708 | 0x2717 | 0x2719 => "G-Wolves HTX Mini",
        0x5408 | 0x5417 | 0x5418 | 0x5419 => "G-Wolves HTS Plus",
        0x5708 | 0x5717 => "G-Wolves HTX",
        0x5908 | 0x5917 => "G-Wolves HSK Plus",
        0x7204 | 0x7203 => "G-Wolves HSK Lite",
        0x7708 | 0x7717 => "G-Wolves HTR Pro",
        0x5804 | 0x5803 => "G-Wolves HSK Pro ACE",
        0x5404 | 0x5403 => "G-Wolves HTS Plus ACE",
        0x5704 | 0x5703 => "G-Wolves HTX ACE",
        0x5904 | 0x5903 => "G-Wolves HSK Plus ACE",
        0x4718 | 0x4719 => "G-Wolves Lycan",
        0x4219 => "G-Wolves WARG",
        0x5219 => "G-Wolves HTS Plus Pro",
        0x3519 => "G-Wolves Fenrir Asym",
        0x2619 => "G-Wolves HTX Mini Asym",
        0x1619 => "G-Wolves Fenrir Max Asym",
        0x2119 => "G-Wolves VUK Mini",
        _ => "G-Wolves wireless mouse",
    }
}

pub fn gwolves_cable(pid: u16) -> bool {
    matches!(
        pid,
        0x1619
            | 0x2119
            | 0x2619
            | 0x2708
            | 0x2719
            | 0x3508
            | 0x3519
            | 0x3608
            | 0x3619
            | 0x3708
            | 0x3808
            | 0x3908
            | 0x4219
            | 0x4718
            | 0x4719
            | 0x5219
            | 0x5308
            | 0x5404
            | 0x5408
            | 0x5418
            | 0x5419
            | 0x5608
            | 0x5618
            | 0x5619
            | 0x5704
            | 0x5708
            | 0x5804
            | 0x5808
            | 0x5904
            | 0x5908
            | 0x6808
            | 0x7204
            | 0x7704
            | 0x7708
            | 0x7904
            | 0x7908
    )
}

fn sum55(b: &[u8]) -> u8 {
    b.iter().fold(0x55u8, |a, &v| a.wrapping_sub(v))
}

/// 17-byte report 08 request: `[08][cmd][0..][check]`, all bytes summing to 0x55.
pub fn report8(cmd: u8) -> Vec<u8> {
    let mut r = vec![0u8; 17];
    r[0] = 0x08;
    r[1] = cmd;
    r[16] = sum55(&r[..16]);
    r
}

/// A report 08 reply to `cmd` with a valid check byte.
pub fn report8_reply(r: &[u8], cmd: u8) -> bool {
    r.len() >= 17 && r[0] == 0x08 && r[1] == cmd && r[16] == sum55(&r[..16])
}

/// Evision request `[04][seq lo][seq hi][1A][sub]`, 32 bytes.
pub fn evision_request(seq: u16, sub: u8) -> Vec<u8> {
    let mut r = vec![0u8; 32];
    r[0] = 0x04;
    r[1..3].copy_from_slice(&seq.to_le_bytes());
    r[3] = 0x1A;
    r[4] = sub;
    r
}

#[derive(Debug, PartialEq)]
pub enum Evision {
    Level(u8, bool),
    /// Our reply, but no usable reading in it.
    Nothing,
}

/// Reply to `seq`: `[04][seq][1A][..][00 00][..][percent][state]`, state 1 or 2 = charging.
/// `busy` also accepts `[04][seq][FF]` as a sleeping keyboard.
pub fn evision_reply(r: &[u8], seq: u16, busy: bool) -> Option<Evision> {
    if r.len() < 4 || r[0] != 0x04 || r[1..3] != seq.to_le_bytes() {
        return None;
    }
    if busy && r[3] == 0xFF {
        return Some(Evision::Nothing);
    }
    if r.len() < 10 || r[3] != 0x1A || r[5] != 0 || r[6] != 0 {
        return None;
    }
    Some(if r[8] > 100 || r[9] > 2 {
        Evision::Nothing
    } else {
        Evision::Level(r[8], r[9] != 0)
    })
}

/// Sends one Evision battery request and waits for its reply.
pub fn evision_query(dev: &Dev, sub: u8, timeout: Duration, busy: bool) -> Option<(u8, bool)> {
    static SEQ: AtomicU16 = AtomicU16::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) % 0xFFFF + 1;
    dev.flush();
    if !dev.write(&evision_request(seq, sub)) {
        return None;
    }
    match dev.read_until(timeout, |r| evision_reply(r, seq, busy))? {
        Evision::Level(p, c) => Some((p, c)),
        Evision::Nothing => None,
    }
}

/// Airoha RACE "get battery" (0x0CD6) for the headset behind the dongle, as output report 06.
pub const RACE_BATTERY: [u8; 10] = [0x06, 0x07, 0x80, 0x05, 0x5A, 0x03, 0x00, 0xD6, 0x0C, 0x00];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report8_checksum_round_trips() {
        let r = report8(0x04);
        assert_eq!(r[16], 0x55u8.wrapping_sub(0x0C));
        assert!(report8_reply(&r, 0x04));
        assert!(!report8_reply(&r, 0x03));
    }

    #[test]
    fn evision_reply_needs_the_echoed_sequence() {
        let mut r = vec![0u8; 31];
        r[..10].copy_from_slice(&[0x04, 0x34, 0x12, 0x1A, 0x02, 0, 0, 0, 0x61, 0x00]);
        assert_eq!(
            evision_reply(&r, 0x1234, false),
            Some(Evision::Level(97, false))
        );
        assert_eq!(evision_reply(&r, 0x1235, false), None);
        r[9] = 2;
        assert_eq!(
            evision_reply(&r, 0x1234, false),
            Some(Evision::Level(97, true))
        );
        r[3] = 0x12;
        assert_eq!(evision_reply(&r, 0x1234, false), None);
        r[3] = 0xFF;
        assert_eq!(evision_reply(&r, 0x1234, true), Some(Evision::Nothing));
        assert_eq!(
            evision_request(0x1234, 0x06)[..5],
            [0x04, 0x34, 0x12, 0x1A, 0x06]
        );
    }

    #[test]
    fn gwolves_tables() {
        assert_eq!(gwolves_name(0x3519), "G-Wolves Fenrir Asym");
        assert_eq!(gwolves_name(0x3854), "G-Wolves wireless mouse");
        assert!(gwolves_cable(0x3519) && !gwolves_cable(0x3517));
    }
}
