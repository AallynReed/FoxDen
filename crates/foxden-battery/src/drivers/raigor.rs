use std::time::Duration;

use crate::hid::Dev;
use crate::util::kind_from_collections;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1EA7;
const REPORT: u8 = 0xB5;

/// Status page chunk `B5 05 94 04 [id][state][%][..]`, or the wake announcement `B5 67 [id x4][state][%]`.
fn parse_frame(r: &[u8]) -> Option<u8> {
    if r.len() != 8 || r[0] != REPORT {
        return None;
    }
    match r[1] {
        0x05 if r[2] == 0x94 && r[3] == 0x04 && r[6] <= 100 => Some(r[6]),
        0x67 if r[7] <= 100 => Some(r[7]),
        _ => None,
    }
}

pub fn poll_raigor(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        if pid != 0x0066 && pid != 0x0064 {
            continue;
        }
        for c in ctx.find(vid, pid, |c| {
            c.page == 0xFFB5 && c.usage == 0x0001 && c.in_len == 8 && c.out_len == 8
        }) {
            let Some(dev) = Dev::open(&c) else { continue };
            dev.flush();
            // Asks for the 256-byte status page, streamed as 8-byte chunks.
            if !dev.write(&[REPORT, 0x05, 0, 0, 0, 0, 0, 0]) {
                continue;
            }
            let Some(pct) = dev.read_until(Duration::from_millis(1200), parse_frame) else {
                continue;
            };
            let kind = match kind_from_collections(&ctx.colls, vid, pid) {
                Kind::Other => Kind::Mouse,
                k => k,
            };
            let b = Battery {
                key: ctx.stable_key(&format!("raigor-{vid:04X}:{pid:04X}")),
                name: if c.product.is_empty() {
                    "2.4 GHz mouse".into()
                } else {
                    c.product.clone()
                },
                via: "2.4 GHz receiver".into(),
                kind,
                percent: Some(pct),
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
    fn status_chunk_and_announcement() {
        assert_eq!(
            parse_frame(&[0xB5, 0x05, 0x94, 0x04, 0x35, 0x09, 0x40, 0x38]),
            Some(64)
        );
        assert_eq!(
            parse_frame(&[0xB5, 0x05, 0x90, 0x04, 0x35, 0x09, 0x40, 0x38]),
            None
        );
        assert_eq!(parse_frame(&[0xB5, 0x67, 1, 2, 3, 4, 9, 55]), Some(55));
        assert_eq!(parse_frame(&[0xB5, 0x67, 1, 2, 3, 4, 9, 155]), None);
    }
}
