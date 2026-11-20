use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x372E;
const RECEIVER: u16 = 0x103D;
const S9_ULTRA_CABLE: u16 = 0x1027;
const REPORT: u8 = 0x03;

/// 64-byte feature request; byte 1 is the low byte of the sum of the bytes after it.
fn request(cmd: u8, sub: u8) -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = REPORT;
    r[2] = 0x50;
    r[5] = cmd;
    r[6] = sub;
    r[1] = r[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    r
}

/// Payload of an acknowledged single-packet reply.
fn payload(r: &[u8]) -> Option<&[u8]> {
    if r.len() < 8 || r[0] != REPORT || r[2] != 0 || r[4] != 0 {
        return None;
    }
    let len = r[3] as usize;
    (len > 0 && 6 + len <= r.len()).then(|| &r[6..6 + len])
}

/// Level byte: bit 7 charging, low bits percent.
fn level(data: &[u8]) -> Option<(u8, bool)> {
    let pct = data.first()? & 0x7F;
    (pct <= 100).then(|| (pct, data[0] & 0x80 != 0))
}

fn query(dev: &Dev, cmd: u8, sub: u8) -> Option<Vec<u8>> {
    if !dev.set_feature(&request(cmd, sub)) {
        return None;
    }
    for _ in 0..3 {
        sleep(Duration::from_millis(100));
        if let Some(r) = dev.get_feature(REPORT)
            && let Some(p) = payload(&r)
        {
            return Some(p.to_vec());
        }
    }
    None
}

pub fn poll_eweadn(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        if pid != RECEIVER && pid != S9_ULTRA_CABLE {
            continue;
        }
        let cable = pid == S9_ULTRA_CABLE;
        for coll in ctx.find(vid, pid, |c| {
            c.page == 0xFF00 && c.usage == 1 && c.feat_len == 64
        }) {
            let Some(dev) = Dev::open_feature_only(&coll) else {
                continue;
            };
            let mut b = Battery {
                key: ctx.stable_key(&format!("eweadn-{vid:04X}:{pid:04X}")),
                name: if cable {
                    "EWEADN S9 Ultra"
                } else {
                    "EWEADN wireless mouse"
                }
                .into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                grace_secs: 600,
                ..Default::default()
            };
            if !cable {
                let Some(online) = query(&dev, 0x47, 0x00) else {
                    continue;
                };
                if online[0] != 1 {
                    b.connected = false;
                    ctx.add(b, vid, pid);
                    break;
                }
            }
            let Some((pct, charging)) = query(&dev, 0x4F, 0x81).as_deref().and_then(level) else {
                continue;
            };
            b.percent = Some(pct);
            b.charging = charging;
            ctx.add(b, vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_checksum_covers_the_bytes_after_it() {
        let r = request(0x4F, 0x81);
        assert_eq!(&r[..7], &[0x03, 0x20, 0x50, 0x00, 0x00, 0x4F, 0x81]);
    }

    #[test]
    fn reply_payload_and_level() {
        let mut r = vec![0u8; 64];
        r[..7].copy_from_slice(&[0x03, 0x50, 0x00, 0x01, 0x00, 0x81, 0xD5]);
        let p = payload(&r).unwrap();
        assert_eq!(level(p), Some((85, true)));
        r[2] = 1;
        assert!(payload(&r).is_none());
        assert_eq!(level(&[0x7F]), None);
    }
}
