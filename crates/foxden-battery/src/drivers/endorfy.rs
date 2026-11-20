use std::thread::sleep;
use std::time::Duration;

use super::vendor_frames::RACE_BATTERY;
use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3738;
const VIRO: u16 = 0x0072;
const REPORT_LEN: u16 = 62;
const IN_ID: u8 = 0x07;
const INDICATION: u8 = 0x5D;
const BATTERY_OP: u16 = 0x0CD6;

fn vendor_collection(c: &Coll) -> bool {
    c.page == 0xFF13 && c.usage == 0x0001 && c.in_len == REPORT_LEN && c.out_len == REPORT_LEN
}

/// Walks the RACE frames `05 [type] [len le] [op le] [payload]` after the 3-byte header of report 07,
/// for the battery indication with payload `[status 0][role][percent]`.
fn battery_percent(r: &[u8]) -> Option<u8> {
    if r.len() < 9 || r[0] != IN_ID {
        return None;
    }
    let mut i = 3;
    while i + 6 <= r.len() {
        if r[i] != 0x05 {
            i += 1;
            continue;
        }
        let len = u16::from_le_bytes([r[i + 2], r[i + 3]]) as usize;
        if len < 2 || i + 4 + len > r.len() {
            return None;
        }
        let op = u16::from_le_bytes([r[i + 4], r[i + 5]]);
        if r[i + 1] == INDICATION
            && op == BATTERY_OP
            && len >= 5
            && r[i + 6] == 0
            && r[i + 8] <= 100
        {
            return Some(r[i + 8]);
        }
        i += 4 + len;
    }
    None
}

fn read_level(c: &Coll) -> Option<u8> {
    const READS: usize = 12;
    let dev = Dev::open(c)?;
    // Report 07 still holds the last answer; don't take it as the new one until the final read.
    let before = dev.get_input_report(IN_ID);
    if !dev.write(&RACE_BATTERY) {
        return None;
    }
    for i in 0..READS {
        sleep(Duration::from_millis(60));
        let Some(r) = dev.get_input_report(IN_ID) else {
            continue;
        };
        if before.as_ref() == Some(&r) && i + 1 < READS {
            continue;
        }
        if let Some(l) = battery_percent(&r) {
            return Some(l);
        }
    }
    None
}

pub fn poll_endorfy(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, VIRO) {
        return;
    }
    let Some(c) = ctx.find(VID, VIRO, vendor_collection).into_iter().next() else {
        return;
    };
    let level = read_level(&c);
    ctx.claim(VID, VIRO);
    let Some(pct) = level else { return };
    let b = Battery {
        key: ctx.stable_key("endorfy-viro"),
        name: "ENDORFY Viro Wireless".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Headset,
        percent: Some(pct),
        grace_secs: 180,
        ..Default::default()
    };
    ctx.add(b, VID, VIRO);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indication_after_the_acknowledgement() {
        let mut r = vec![0u8; 62];
        let frames = [
            0x07, 0x11, 0x80, // header
            0x05, 0x5B, 0x03, 0x00, 0xD6, 0x0C, 0x00, // acknowledgement
            0x05, 0x5D, 0x05, 0x00, 0xD6, 0x0C, 0x00, 0x00, 0x3C, // indication: 60%
        ];
        r[..frames.len()].copy_from_slice(&frames);
        assert_eq!(battery_percent(&r), Some(60));
        r[11] = 0x5B;
        assert_eq!(battery_percent(&r), None);
    }
}
