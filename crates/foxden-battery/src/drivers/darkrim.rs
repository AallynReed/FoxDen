use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use crate::hid::Dev;
use crate::util::crc16_ccitt;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3654;
const PID: u16 = 0x6901;

/// GET 0x0010 for TLV 01 (battery): `FE seq [cmd le] [type] [index] [count] [len le] [payload] [crc be]`, behind report id 0.
fn request(seq: u8) -> Vec<u8> {
    let frame = [
        0xFE, seq, 0x10, 0x00, 0x01, 0x00, 0x01, 0x02, 0x00, 0x01, 0x00,
    ];
    let mut r = vec![0x00];
    r.extend_from_slice(&frame);
    r.extend_from_slice(&crc16_ccitt(0xFFFF, &frame).to_be_bytes());
    r
}

/// The response to `seq`: walks its TLVs for type 01 (bit 7 charging, low bits percent).
fn parse_reply(r: &[u8], seq: u8) -> Option<(u8, bool)> {
    if r.len() < 11
        || r[0] != 0xFE
        || r[1] != seq
        || r[2..6] != [0x10, 0x00, 0x02, 0x00]
        || r[6] > 1
    {
        return None;
    }
    let len = u16::from_le_bytes([r[7], r[8]]) as usize;
    if len > 53 || len + 11 > r.len() {
        return None;
    }
    let end = 9 + len;
    if crc16_ccitt(0xFFFF, &r[..end]) != u16::from_be_bytes([r[end], r[end + 1]]) {
        return None;
    }
    let mut tlvs = &r[9..end];
    let mut level = None;
    while !tlvs.is_empty() {
        let [kind, size, rest @ ..] = tlvs else {
            return None;
        };
        let size = *size as usize;
        if size > rest.len() {
            return None;
        }
        if *kind == 1 {
            if size != 1 || level.is_some() || rest[0] & 0x7F > 100 {
                return None;
            }
            level = Some(rest[0]);
        }
        tlvs = &rest[size..];
    }
    level.map(|v| (v & 0x7F, v & 0x80 != 0))
}

pub fn poll_darkrim(ctx: &mut Ctx) {
    static SEQ: AtomicU8 = AtomicU8::new(0);
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.iface == 3 && c.page == 0xFF02 && c.usage == 2 && c.in_len == 65 && c.out_len == 65
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        dev.flush();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed) % 15 + 1;
        if !dev.write_timeout(&request(seq), Duration::from_millis(200)) {
            continue;
        }
        // Startup notifications come first; the answer takes about 400 ms.
        let Some((pct, charging)) =
            dev.read_until(Duration::from_millis(800), |r| parse_reply(r, seq))
        else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key("darkrim-shadow"),
            name: "DARKRIM SHADOW".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            percent: Some(pct),
            charging,
            ..Default::default()
        };
        ctx.add(b, VID, PID);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(seq: u8, tlvs: &[u8]) -> Vec<u8> {
        let mut r = vec![0xFE, seq, 0x10, 0x00, 0x02, 0x00, 0x00];
        r.extend_from_slice(&(tlvs.len() as u16).to_le_bytes());
        r.extend_from_slice(tlvs);
        let crc = crc16_ccitt(0xFFFF, &r);
        r.extend_from_slice(&crc.to_be_bytes());
        r.resize(64, 0);
        r
    }

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc16_ccitt(0xFFFF, b"123456789"), 0x29B1);
        let r = request(3);
        assert_eq!(r[..3], [0x00, 0xFE, 0x03]);
        assert_eq!(crc16_ccitt(0xFFFF, &r[1..12]).to_be_bytes(), [r[12], r[13]]);
    }

    #[test]
    fn battery_tlv() {
        assert_eq!(
            parse_reply(&reply(3, &[0x01, 0x01, 0xD5]), 3),
            Some((85, true))
        );
        assert_eq!(
            parse_reply(&reply(3, &[0x02, 0x02, 9, 9, 0x01, 0x01, 0x40]), 3),
            Some((64, false))
        );
        assert_eq!(parse_reply(&reply(3, &[0x01, 0x01, 0x40]), 4), None);
        let mut bad = reply(3, &[0x01, 0x01, 0x40]);
        bad[9] ^= 0xFF;
        assert_eq!(parse_reply(&bad, 3), None);
    }
}
