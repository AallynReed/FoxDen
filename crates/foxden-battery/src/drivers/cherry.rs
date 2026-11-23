use std::time::Duration;

use super::oem_frames::{frame55, is_status1a_coll, query_status1a, sum55_ok};
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x046A;
const XTRFY_RECEIVER: u16 = 0x0330;

pub fn poll_cherry(ctx: &mut Ctx) {
    mx30s(ctx);
    xtrfy(ctx);
}

/// MX 3.0S Wireless: the cable is tried before the dongle so the keyboard is listed once.
fn mx30s(ctx: &mut Ctx) {
    for (pid, via) in [(0x00E3, "USB cable"), (0x00EA, "2.4 GHz receiver")] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for c in ctx.find(VID, pid, is_status1a_coll) {
            let Some((level, charging)) = query_status1a(&c, |_| true) else {
                continue;
            };
            let known = level <= 100;
            let b = Battery {
                key: ctx.stable_key("cherry-mx-3.0s"),
                name: "CHERRY MX 3.0S Wireless".into(),
                via: via.into(),
                kind: Kind::Keyboard,
                percent: known.then_some(level),
                charging: known && charging,
                asleep: !known,
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, VID, pid);
            return;
        }
    }
}

#[derive(Debug, PartialEq)]
struct XtrfyPower {
    percent: Option<u8>,
    charging: bool,
    millivolts: u32,
}

fn parse_xtrfy_power(r: &[u8]) -> XtrfyPower {
    XtrfyPower {
        percent: (r[6] <= 100).then_some(r[6]),
        charging: r[7] != 0,
        millivolts: u16::from_be_bytes([r[8], r[9]]) as u32,
    }
}

fn xtrfy_exchange(dev: &Dev, op: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write_timeout(&frame55(0x08, op), Duration::from_millis(300)) {
        return None;
    }
    dev.read_until(Duration::from_millis(500), |r| {
        (r.len() == 17 && r[0] == 0x08 && r[1] == op && sum55_ok(r)).then(|| r.to_vec())
    })
}

fn xtrfy(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, XTRFY_RECEIVER) {
        return;
    }
    let colls = ctx.find(VID, XTRFY_RECEIVER, |c| {
        c.page == 0xFF02 && c.usage == 0x0002 && c.in_len == 17 && c.out_len == 17
    });
    let Some(dev) = colls.first().and_then(Dev::open) else {
        return;
    };
    // 03 = radio link, 04 = power
    let link = xtrfy_exchange(&dev, 0x03).map(|r| r[6] != 0);
    if link == Some(false) {
        ctx.claim(VID, XTRFY_RECEIVER);
        return;
    }
    let mut b = Battery {
        name: "Cherry XTRFY wireless mouse".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Mouse,
        grace_secs: 1800,
        ..Default::default()
    };
    match xtrfy_exchange(&dev, 0x04) {
        Some(r) => {
            let p = parse_xtrfy_power(&r);
            b.percent = p.percent;
            b.charging = p.charging;
        }
        None if link.is_some() => b.asleep = true,
        None => return,
    }
    b.key = ctx.stable_key("cherry-xtrfy");
    ctx.add(b, VID, XTRFY_RECEIVER);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xtrfy_power_frame() {
        let r = [
            0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x5F, 0x00, 0x0F, 0xF4, 0, 0, 0, 0, 0, 0, 0xE5,
        ];
        assert!(sum55_ok(&r));
        assert_eq!(
            parse_xtrfy_power(&r),
            XtrfyPower {
                percent: Some(95),
                charging: false,
                millivolts: 4084
            }
        );
    }
}
