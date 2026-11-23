use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x260D;
const FRAME: usize = 17;

/// Status request on report 08: command 04, last byte makes the frame sum 0x55.
pub(super) fn request() -> [u8; FRAME] {
    let mut f = [0u8; FRAME];
    f[0] = 0x08;
    f[1] = 0x04;
    let sum = f[..16].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    f[16] = 0x55u8.wrapping_sub(sum);
    f
}

#[derive(Debug, PartialEq)]
struct Status {
    percent: u8,
    charging: bool,
    millivolts: u32,
}

fn parse(r: &[u8]) -> Option<Status> {
    if r.len() != FRAME || r[0] != 0x08 || r[1] != 0x04 || r[6] > 100 {
        return None;
    }
    if r.iter().fold(0u8, |a, &b| a.wrapping_add(b)) != 0x55 {
        return None;
    }
    Some(Status {
        percent: r[6],
        charging: r[7] != 0,
        millivolts: u32::from(u16::from_be_bytes([r[8], r[9]])),
    })
}

fn poll_one(ctx: &mut Ctx, pid: u16, via: &str) {
    if ctx.is_claimed(VID, pid) {
        return;
    }
    let colls = ctx.find(VID, pid, |c| {
        c.iface == 1 && c.page == 0xFF02 && c.usage == 2 && c.in_len == 17 && c.out_len == 17
    });
    let [coll] = colls.as_slice() else { return };
    let Some(dev) = Dev::open(coll) else { return };
    dev.flush();
    if !dev.write(&request()) {
        return;
    }
    let Some(st) = dev.read_until(Duration::from_secs(1), parse) else {
        return;
    };
    let b = Battery {
        key: ctx.stable_key(&format!("zenith-{VID:04X}:{pid:04X}")),
        name: "Zenith Pro".into(),
        via: via.into(),
        kind: Kind::Mouse,
        percent: Some(st.percent),
        charging: st.charging,
        grace_secs: 1800,
        ..Default::default()
    };
    ctx.add(b, VID, pid);
}

pub fn poll_zenith(ctx: &mut Ctx) {
    poll_one(ctx, 0x1114, "2.4 GHz receiver");
    poll_one(ctx, 0x1143, "USB cable");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_sums_to_0x55() {
        let r = request();
        assert_eq!(r[16], 0x49);
        assert_eq!(r.iter().fold(0u8, |a, &b| a.wrapping_add(b)), 0x55);
    }

    #[test]
    fn status_reply_parses() {
        let mut r = [0u8; FRAME];
        r[..10].copy_from_slice(&[0x08, 0x04, 0, 0, 0, 0, 77, 1, 0x0F, 0xA0]);
        let sum = r[..16].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        r[16] = 0x55u8.wrapping_sub(sum);
        assert_eq!(
            parse(&r),
            Some(Status {
                percent: 77,
                charging: true,
                millivolts: 4000
            })
        );
        r[16] ^= 1;
        assert_eq!(parse(&r), None);
    }
}
