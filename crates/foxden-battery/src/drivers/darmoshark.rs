use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const TELINK: u16 = 0x248A;
const FEATURE: u8 = 0x51;
const BOND: u8 = 0x03;
const IDENTIFY: u8 = 0x06;

/// (receiver pid, mouse pid, name).
const TELINK_MODELS: &[(u16, u16, &str)] = &[
    (0xFF30, 0xFF12, "Darmoshark M3"),
    (0xFF31, 0xFF18, "Darmoshark M3S"),
];

fn le16(r: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([r[at], r[at + 1]])
}

/// The bond reply names the paired mouse and whether it is bonded.
fn bonded(r: &[u8], mouse: u16) -> bool {
    r.len() >= 12 && le16(r, 3) == TELINK && le16(r, 5) == mouse && r[7] != 0
}

/// The identify reply carries the mouse ids and its percent at byte 11.
fn identify_level(r: &[u8], mouse: u16) -> Option<u8> {
    (r.len() >= 12 && r[2] == 1 && le16(r, 4) == TELINK && le16(r, 6) == mouse && r[11] <= 100)
        .then(|| r[11])
}

/// Receiver input `54 E4 02`: the request was refused.
fn refused(r: &[u8]) -> bool {
    r.len() >= 3 && r[..3] == [0x54, 0xE4, 0x02]
}

fn query(dev: &Dev, op: u8, limit: Instant) -> Option<Vec<u8>> {
    let end = limit.min(Instant::now() + Duration::from_millis(500));
    dev.flush();
    if Instant::now() >= end || !dev.set_feature(&[FEATURE, op]) {
        return None;
    }
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if dev
            .read(left.min(Duration::from_millis(20)))
            .is_some_and(|r| refused(&r))
        {
            return None;
        }
        if let Some(r) = dev.get_feature(FEATURE)
            && r.len() >= 12
            && r[0] == FEATURE
            && r[1] == op
        {
            return (Instant::now() < end).then_some(r);
        }
    }
    None
}

fn read_level(dev: &Dev, mouse: u16, limit: Instant) -> Option<u8> {
    // The bond query also clears an older reply from the receiver's buffer.
    if !bonded(&query(dev, BOND, limit)?, mouse) {
        return None;
    }
    identify_level(&query(dev, IDENTIFY, limit)?, mouse)
}

pub fn poll_darmoshark(ctx: &mut Ctx) {
    for &(receiver, mouse, name) in TELINK_MODELS {
        if ctx.is_claimed(TELINK, receiver) {
            continue;
        }
        let colls = ctx.find(TELINK, receiver, |c| {
            c.iface == 1
                && c.page == 0x008C
                && c.usage == 1
                && c.in_len == 33
                && c.out_len == 33
                && c.feat_len == 1001
        });
        for c in colls {
            let limit = Instant::now() + Duration::from_millis(1000);
            let Some(dev) = Dev::open(&c) else { continue };
            let first = limit.min(Instant::now() + Duration::from_millis(300));
            let Some(pct) =
                read_level(&dev, mouse, first).or_else(|| read_level(&dev, mouse, limit))
            else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&format!("darmoshark-{TELINK:04X}:{receiver:04X}")),
                name: name.into(),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Mouse,
                percent: Some(pct),
                ..Default::default()
            };
            ctx.add(b, TELINK, receiver);
            break;
        }
    }
}

const NRF: u16 = 0x1915;
const NRF_RECEIVER: u16 = 0x0723;
const NRF_FRAME: usize = 64;

/// Last byte that makes all 64 bytes sum to 0xA1.
fn nrf_check(frame: &[u8]) -> u8 {
    frame[..NRF_FRAME - 1]
        .iter()
        .fold(0xA1u8, |a, &b| a.wrapping_sub(b))
}

fn nrf_request() -> Vec<u8> {
    let mut f = vec![0u8; NRF_FRAME];
    f[..4].copy_from_slice(&[0x41, 0x00, 0x81, 0x01]);
    f[NRF_FRAME - 1] = nrf_check(&f);
    let mut r = vec![0x00];
    r.extend_from_slice(&f);
    r
}

/// Reply `41 .. 8C 01 .. .. [percent]`, checksummed.
fn nrf_level(r: &[u8]) -> Option<u8> {
    if r.len() < NRF_FRAME || r[0] != 0x41 || r[2] != 0x8C || r[3] != 0x01 {
        return None;
    }
    (r[NRF_FRAME - 1] == nrf_check(r) && r[6] <= 100).then_some(r[6])
}

pub fn poll_darmoshark_nrf(ctx: &mut Ctx) {
    if ctx.is_claimed(NRF, NRF_RECEIVER) {
        return;
    }
    let colls = ctx.find(NRF, NRF_RECEIVER, |c| {
        c.iface == 3 && c.page == 0xFF0A && c.usage == 1 && c.in_len == 65 && c.out_len == 65
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        dev.flush();
        if !dev.write(&nrf_request()) {
            continue;
        }
        let Some(pct) = dev.read_until(Duration::from_millis(600), nrf_level) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("darmoshark-{NRF:04X}:{NRF_RECEIVER:04X}")),
            name: "Darmoshark M3S Pro".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, NRF, NRF_RECEIVER);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telink_bond_and_identify() {
        let mut bond = vec![0u8; 12];
        bond[..8].copy_from_slice(&[0x51, 0x03, 0x00, 0x8A, 0x24, 0x18, 0xFF, 0x01]);
        assert!(bonded(&bond, 0xFF18));
        assert!(!bonded(&bond, 0xFF12));
        let mut id = vec![0u8; 12];
        id[..8].copy_from_slice(&[0x51, 0x06, 0x01, 0x00, 0x8A, 0x24, 0x18, 0xFF]);
        id[11] = 72;
        assert_eq!(identify_level(&id, 0xFF18), Some(72));
        id[2] = 0;
        assert_eq!(identify_level(&id, 0xFF18), None);
        assert!(refused(&[0x54, 0xE4, 0x02, 0x00]));
    }

    #[test]
    fn nrf_frames_sum_to_a1() {
        let req = nrf_request();
        assert_eq!(req.len(), 65);
        assert_eq!(req[1..].iter().fold(0u8, |a, &b| a.wrapping_add(b)), 0xA1);
        let mut r = vec![0u8; 64];
        r[..7].copy_from_slice(&[0x41, 0x00, 0x8C, 0x01, 0, 0, 58]);
        r[63] = nrf_check(&r);
        assert_eq!(nrf_level(&r), Some(58));
        r[63] ^= 1;
        assert_eq!(nrf_level(&r), None);
    }
}
