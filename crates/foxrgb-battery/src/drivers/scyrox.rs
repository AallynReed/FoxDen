use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x395A;
const V12M_RECEIVER: u16 = 0x0006;

#[derive(Debug, PartialEq)]
enum Frame {
    Other,
    End,
    Battery(u8),
}

/// One frame of the profile stream answering command 01; the active profile carries the battery at [20].
fn classify(r: &[u8]) -> Frame {
    if r.len() != 64 || r[0] != 0x08 || r[1] != 0x01 {
        return Frame::Other;
    }
    if r[3] == 0 {
        return Frame::End;
    }
    let active = r[62] != 1 && r[4] <= 3 && r[4] == r[5] && (1..=5).contains(&r[7]);
    match (active, r[20]) {
        (false, _) => Frame::Other,
        (true, pct) if pct <= 100 => Frame::Battery(pct),
        _ => Frame::End,
    }
}

fn read_battery(dev: &Dev) -> Option<u8> {
    let mut req = [0u8; 64];
    req[0] = 0x08;
    // read profiles; [63] = low byte of 08 + 01
    req[1] = 0x01;
    req[63] = 0x09;
    dev.flush();
    if !dev.write(&req) {
        return None;
    }
    let pct = dev
        .read_until(Duration::from_millis(1500), |r| match classify(r) {
            Frame::Other => None,
            Frame::End => Some(None),
            Frame::Battery(p) => Some(Some(p)),
        })
        .flatten();
    dev.flush();
    pct
}

pub fn poll_scyrox(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, V12M_RECEIVER) {
        return;
    }
    let colls = ctx.find(VID, V12M_RECEIVER, |c| {
        c.page == 0xFF00 && c.usage == 0x0002 && c.out_len == 64 && c.in_len == 64
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        let Some(pct) = read_battery(&dev) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key("scyrox-395A:0006"),
            name: "Scyrox V12M".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, V12M_RECEIVER);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_profile_frame() {
        let mut r = [0u8; 64];
        r[..8].copy_from_slice(&[0x08, 0x01, 0x00, 0x01, 0x02, 0x02, 0x00, 0x04]);
        r[20] = 66;
        assert_eq!(classify(&r), Frame::Battery(66));
        r[5] = 1;
        assert_eq!(classify(&r), Frame::Other);
        r[3] = 0;
        assert_eq!(classify(&r), Frame::End);
    }
}
