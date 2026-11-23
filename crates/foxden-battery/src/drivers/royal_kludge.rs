use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x2E3C;
const CABLES: [u16; 2] = [0x00A5, 0x00E5];
const RECEIVERS: [u16; 2] = [0x00A6, 0x00E6];

/// Forwarded to the mouse: system control, get battery voltage, read access.
const REQUEST: [u8; 5] = [0x00, 0x01, 0x01, 0x02, 0x01];

#[derive(Debug, PartialEq)]
enum Reply {
    /// The receiver answered for a mouse that didn't.
    NoMouse,
    Level {
        percent: u8,
        mv: u32,
        usb: bool,
    },
}

/// `None` for unrelated notifications on the same collection.
fn parse(r: &[u8]) -> Option<Option<Reply>> {
    if r.len() < 11 || r[0] & 0x1F != 0x01 || r[1] & 0x0F != 0x01 || r[2] != 0x02 {
        return None;
    }
    if r[3] & 0x10 == 0 {
        return Some(Some(Reply::NoMouse));
    }
    let percent = u16::from_le_bytes([r[6], r[7]]);
    if percent > 100 {
        return Some(None);
    }
    Some(Some(Reply::Level {
        percent: percent as u8,
        mv: u16::from_le_bytes([r[4], r[5]]) as u32,
        usb: r[10] != 0,
    }))
}

fn query(dev: &Dev) -> Option<Reply> {
    let mut answered = None;
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(Duration::from_millis(100));
        }
        dev.flush();
        if !dev.write(&REQUEST) {
            break;
        }
        match dev.read_until(Duration::from_millis(600), parse) {
            Some(Some(r @ Reply::Level { .. })) => return Some(r),
            Some(_) => answered = Some(Reply::NoMouse),
            None => {}
        }
    }
    answered
}

pub fn poll_royal_kludge(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        let cable = CABLES.contains(&pid);
        if !cable && !RECEIVERS.contains(&pid) {
            continue;
        }
        let page = if cable { 0xFFF2 } else { 0xFF00 };
        let colls = ctx.find(VID, pid, |c| {
            c.page == page && c.usage == 1 && c.out_len == 65 && c.in_len == 65
        });
        for c in colls {
            let Some(reply) = Dev::open(&c).and_then(|d| query(&d)) else {
                continue;
            };
            let mut b = Battery {
                key: ctx.stable_key(&format!("rk-{}:{}", hex4(VID), hex4(pid))),
                name: "RK MG6 Pro".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                ..Default::default()
            };
            match reply {
                Reply::Level { percent, usb, .. } => {
                    b.percent = Some(percent);
                    b.charging = usb || cable;
                }
                Reply::NoMouse => b.asleep = true,
            }
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_reply() {
        let r = [0x01, 0x01, 0x02, 0x11, 0x2C, 0x0F, 77, 0, 25, 0, 1];
        assert_eq!(
            parse(&r),
            Some(Some(Reply::Level {
                percent: 77,
                mv: 3884,
                usb: true
            }))
        );
    }

    #[test]
    fn mouse_silent_or_unrelated() {
        assert_eq!(
            parse(&[0x01, 0x01, 0x02, 0x01, 0, 0, 0, 0, 0, 0, 0]),
            Some(Some(Reply::NoMouse))
        );
        assert_eq!(parse(&[0x03, 0x01, 0x02, 0x11, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(
            parse(&[0x01, 0x01, 0x02, 0x11, 0, 0, 101, 0, 0, 0, 0]),
            Some(None)
        );
    }
}
