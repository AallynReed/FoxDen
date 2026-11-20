use std::thread::sleep;
use std::time::{Duration, Instant};

use super::vendor_frames::RACE_BATTERY;
use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x0ECB;
const N9_HYBRID: u16 = 0x20D9;
const REPLY: [u8; 7] = [0x05, 0x5D, 0x05, 0x00, 0xD6, 0x0C, 0x00];

/// Newest battery reply `05 5D 05 00 D6 0C [00][agent][percent]` in the valid part of report 07.
/// `[1]` counts the valid bytes from `[3]`; the tail can hold stale frames from before the headset went off.
fn find_level(r: &[u8]) -> Option<u8> {
    if r.len() < 4 {
        return None;
    }
    let valid = r.len().min(3 + r[1] as usize);
    r[3..valid]
        .windows(9)
        .rev()
        .find(|w| w[..7] == REPLY && w[8] <= 100)
        .map(|w| w[8])
}

fn read_level(c: &Coll) -> Option<u8> {
    let dev = Dev::open(c)?;
    if !dev.write(&RACE_BATTERY) {
        return None;
    }
    let end = Instant::now() + Duration::from_millis(600);
    while Instant::now() < end {
        sleep(Duration::from_millis(15));
        if let Some(l) = dev.get_input_report(0x07).as_deref().and_then(find_level) {
            return Some(l);
        }
    }
    None
}

pub fn poll_akg(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, N9_HYBRID) {
        return;
    }
    let colls = ctx.find(VID, N9_HYBRID, |c| {
        c.iface == 3 && c.page == 0xFF13 && c.usage == 0x0001 && c.in_len == 62 && c.out_len == 62
    });
    for c in colls {
        let Some(pct) = read_level(&c) else { continue };
        let b = Battery {
            key: ctx.stable_key(&format!("akg-{VID:04X}:{N9_HYBRID:04X}")),
            name: "AKG N9 Hybrid".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, VID, N9_HYBRID);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_valid_part_counts() {
        let mut r = vec![0u8; 62];
        r[..3].copy_from_slice(&[0x07, 0x09, 0x80]);
        r[3..12].copy_from_slice(&[0x05, 0x5D, 0x05, 0x00, 0xD6, 0x0C, 0x00, 0x00, 0x32]);
        assert_eq!(find_level(&r), Some(50));
        r[1] = 0x08;
        assert_eq!(find_level(&r), None);
        let mut stale = vec![0u8; 62];
        stale[..3].copy_from_slice(&[0x07, 0x04, 0x80]);
        stale[30..39].copy_from_slice(&[0x05, 0x5D, 0x05, 0x00, 0xD6, 0x0C, 0x00, 0x00, 0x32]);
        assert_eq!(find_level(&stale), None);
    }
}
