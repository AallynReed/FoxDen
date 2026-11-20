use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x413C;
const AW920H: u16 = 0xA515;
const REQUEST: [u8; 3] = [0x06, 0xC0, 0x0A];
const REPLY_ID: u8 = 0x07;

fn vendor_collection(c: &Coll) -> bool {
    c.iface == 5 && c.page == 0xFF13 && c.usage == 0x0001 && c.in_len == 62 && c.out_len == 62
}

/// Reply `[07][C0][0A][09][..][linked][%][charging][critical][..][..][critical %][mode][xor of 3..=12]`.
fn parse_reply(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 14 || r[..4] != [REPLY_ID, 0xC0, 0x0A, 0x09] {
        return None;
    }
    if r[3..13].iter().fold(0, |a, &b| a ^ b) != r[13] || r[6] == 0 {
        return None;
    }
    let pct = if r[8] == 1 { r[11] } else { r[6] };
    (pct <= 100).then_some((pct, r[7] == 1))
}

fn query(dev: &Dev) -> Option<(u8, bool)> {
    let end = Instant::now() + Duration::from_millis(600);
    // About one request in a hundred is dropped, so it is resent.
    for _ in 0..3 {
        if Instant::now() >= end {
            break;
        }
        if let Some(b) = dev
            .set_output_and_read(&REQUEST, Duration::from_millis(20))
            .as_deref()
            .and_then(parse_reply)
        {
            return Some(b);
        }
        let attempt_end = end.min(Instant::now() + Duration::from_millis(180));
        loop {
            sleep(Duration::from_millis(15));
            if let Some(b) = dev
                .get_input_report(REPLY_ID)
                .as_deref()
                .and_then(parse_reply)
            {
                return Some(b);
            }
            if Instant::now() >= attempt_end {
                break;
            }
        }
    }
    None
}

pub fn poll_alienware(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, AW920H) {
        return;
    }
    for c in ctx.find(VID, AW920H, vendor_collection) {
        let Some(dev) = Dev::open(&c) else { continue };
        let Some((pct, charging)) = query(&dev) else {
            // Headset off or out of range: leave it out, but keep other drivers off the dongle.
            ctx.claim(VID, AW920H);
            break;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("alienware-{VID:04X}:{AW920H:04X}")),
            name: "Alienware AW920H".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            percent: Some(pct),
            charging,
            grace_secs: 180,
            ..Default::default()
        };
        ctx.add(b, VID, AW920H);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_reply() {
        let r = [
            0x07, 0xC0, 0x0A, 0x09, 0x00, 0x01, 0x5F, 0x00, 0x00, 0x1C, 0x10, 0x5F, 0x02, 0x06,
        ];
        assert_eq!(parse_reply(&r), Some((95, false)));
        let mut bad = r;
        bad[13] = 0x07;
        assert_eq!(parse_reply(&bad), None);
        assert_eq!(
            parse_reply(&[0x07, 0x00, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            None
        );
    }
}
