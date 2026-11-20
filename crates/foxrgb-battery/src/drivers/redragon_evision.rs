use std::time::Duration;

use super::vendor_frames::evision_query;
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x320F;
const RECEIVER: u16 = 0x226F;
/// Sub-command the vendor software sends on this receiver.
const SUB: u8 = 0x02;

pub fn poll_redragon_evision(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, RECEIVER) {
        return;
    }
    let Some(c) = ctx
        .find(VID, RECEIVER, |c| c.page == 0xFF1C && c.usage == 0x0092)
        .into_iter()
        .next()
    else {
        return;
    };
    let level =
        Dev::open(&c).and_then(|d| evision_query(&d, SUB, Duration::from_millis(1000), false));
    let b = Battery {
        key: ctx.stable_key(&format!("redragon-{VID:04X}:{RECEIVER:04X}")),
        // The receiver is shared by several models.
        name: "Redragon wireless mouse".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Mouse,
        percent: level.map(|l| l.0),
        charging: level.is_some_and(|l| l.1),
        asleep: level.is_none(),
        grace_secs: 1800,
        ..Default::default()
    };
    ctx.add(b, VID, RECEIVER);
}

#[cfg(test)]
mod tests {
    use super::super::vendor_frames::{Evision, evision_reply, evision_request};
    use super::*;

    #[test]
    fn m999_reply_reads_97_percent() {
        assert_eq!(evision_request(7, SUB)[..5], [0x04, 0x07, 0x00, 0x1A, 0x02]);
        let mut r = vec![0u8; 31];
        r[..10].copy_from_slice(&[0x04, 0x07, 0x00, 0x1A, 0x02, 0, 0, 0, 0x61, 0x00]);
        assert_eq!(evision_reply(&r, 7, false), Some(Evision::Level(97, false)));
    }
}
