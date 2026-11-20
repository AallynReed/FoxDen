use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x352F;
const PD200W: u16 = 0x040E;
const REPORT: u8 = 0xC4;
const GET: u8 = 0x04;
const BATTERY_REG: u16 = 0x0042;

/// Two's complement of the 16-bit byte sum.
fn checksum(b: &[u8]) -> u16 {
    b.iter()
        .fold(0u16, |a, &v| a.wrapping_add(v as u16))
        .wrapping_neg()
}

/// Register read `[C4][len][00 00][04][reg le][check le]`.
fn request() -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = REPORT;
    r[1] = 9;
    r[4] = GET;
    r[5..7].copy_from_slice(&BATTERY_REG.to_le_bytes());
    let c = checksum(&r[..7]);
    r[7..9].copy_from_slice(&c.to_le_bytes());
    r
}

/// Reply `[C4][len][00 00][04]` then `(reg, value)` pairs, little-endian, and the checksum.
fn parse_reply(r: &[u8]) -> Option<u8> {
    if r.len() < 11 || r[0] != REPORT || r[2..5] != [0x00, 0x00, GET] {
        return None;
    }
    let len = r[1] as usize;
    if len < 11 || len > r.len() {
        return None;
    }
    let body = len - 2;
    if !(body - 5).is_multiple_of(4)
        || u16::from_le_bytes([r[body], r[body + 1]]) != checksum(&r[..body])
    {
        return None;
    }
    let value = r[5..body]
        .chunks_exact(4)
        .find(|p| u16::from_le_bytes([p[0], p[1]]) == BATTERY_REG)
        .map(|p| u16::from_le_bytes([p[2], p[3]]))?;
    u8::try_from(value).ok().filter(|&v| v <= 100)
}

fn read_level(dev: &Dev) -> Option<u8> {
    let end = Instant::now() + Duration::from_millis(1000);
    dev.flush();
    if !dev.write_timeout(&request(), Duration::from_millis(250)) {
        return None;
    }
    dev.read_until(end.saturating_duration_since(Instant::now()), parse_reply)
}

pub fn poll_maono(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PD200W) {
        return;
    }
    let colls = ctx.find(VID, PD200W, |c| {
        c.iface == 3
            && c.page == 0x000C
            && c.usage == 0x0001
            && c.in_len == 64
            && c.out_len == 64
            && c.feat_len == 0
    });
    if colls.is_empty() {
        return;
    }
    // The receiver stays listed with the mic off; keep the generic reader from showing a bogus 0%.
    ctx.claim(VID, PD200W);
    for c in colls {
        let Some(pct) = Dev::open(&c).and_then(|d| read_level(&d)) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key("maono-pd200w"),
            name: "MAONO PD200W".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Microphone,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, VID, PD200W);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_frame() {
        assert_eq!(
            request()[..9],
            [0xC4, 0x09, 0x00, 0x00, 0x04, 0x42, 0x00, 0xED, 0xFE]
        );
    }

    #[test]
    fn register_pairs() {
        let mut r = vec![0u8; 64];
        r[..9].copy_from_slice(&[0xC4, 0x0B, 0x00, 0x00, 0x04, 0x42, 0x00, 0x55, 0x00]);
        let c = checksum(&r[..9]);
        r[9..11].copy_from_slice(&c.to_le_bytes());
        assert_eq!(r[9..11], [0x96, 0xFE]);
        assert_eq!(parse_reply(&r), Some(85));
        r[9] ^= 1;
        assert_eq!(parse_reply(&r), None);
    }
}
