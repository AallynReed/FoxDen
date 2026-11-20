use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x5253;
const VID_REALTEK: u16 = 0x3837;
/// Only these 3837 dongles speak this protocol; other 3837 products must not get it.
const REALTEK_PIDS: [u16; 5] = [0x100C, 0x100B, 0x100A, 0x1001, 0x1012];
const REPORT: u8 = 0x12;
const STATUS: u8 = 0x06;

#[derive(Debug, PartialEq)]
enum Status {
    /// Echo with an empty payload: the mouse sleeps.
    Asleep,
    Awake {
        level: u8,
        charging: bool,
        cable: bool,
    },
}

/// Every payload byte travels inverted; `vid_fields` are the accepted vendor ids inside it.
fn parse(r: &[u8], vid_fields: [u16; 2]) -> Option<Status> {
    if r.len() < 13 || r[0] != REPORT || r[1] != !STATUS {
        return None;
    }
    let p: Vec<u8> = r[2..13].iter().map(|b| !b).collect();
    if p.iter().all(|&b| b == 0) {
        return Some(Status::Asleep);
    }
    if !vid_fields.contains(&u16::from_le_bytes([p[0], p[1]])) || p[9] > 100 {
        return None;
    }
    let charging = p[10] != 0;
    Some(Status::Awake {
        level: p[9],
        charging,
        cable: p[8] == 0 && charging,
    })
}

fn request(len: usize) -> Vec<u8> {
    let mut req = vec![0xFF; len];
    req[0] = REPORT;
    req[1] = !STATUS;
    req
}

fn query(dev: &Dev, vid_fields: [u16; 2]) -> Option<Status> {
    let req = request(dev.coll.feat_len as usize);
    let mut best = None;
    for attempt in 0..5 {
        if attempt > 0 {
            sleep(Duration::from_millis(100));
        }
        // Each read needs a fresh request, or the device hands back the request itself.
        if !dev.set_feature(&req) {
            break;
        }
        sleep(Duration::from_millis(120));
        match dev.get_feature(REPORT).and_then(|r| parse(&r, vid_fields)) {
            Some(s @ Status::Awake { .. }) => return Some(s),
            Some(s) => best = Some(s),
            None => {}
        }
    }
    best
}

fn poll_product(ctx: &mut Ctx, vid: u16, pid: u16, vid_fields: [u16; 2]) {
    let colls = ctx.find(vid, pid, |c| {
        c.page == 0xFF01 && c.usage == 0x0001 && c.feat_len >= 65
    });
    for c in colls {
        let Some(status) = Dev::open_feature_only(&c).and_then(|d| query(&d, vid_fields)) else {
            continue;
        };
        let mut b = Battery {
            key: ctx.stable_key(&format!("mchose-{}:{}", hex4(vid), hex4(pid))),
            name: if c.product.is_empty() {
                "MCHOSE mouse".into()
            } else {
                c.product.clone()
            },
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            ..Default::default()
        };
        match status {
            Status::Awake {
                level,
                charging,
                cable,
            } => {
                b.percent = Some(level);
                b.charging = charging;
                if cable {
                    b.via = "USB cable".into();
                }
            }
            Status::Asleep => b.asleep = true,
        }
        ctx.add(b, vid, pid);
        return;
    }
}

pub fn poll_mchose(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        poll_product(ctx, VID, pid, [VID, VID]);
    }
    for pid in REALTEK_PIDS {
        if !ctx.is_claimed(VID_REALTEK, pid) {
            poll_product(ctx, VID_REALTEK, pid, [0x41E4, VID_REALTEK]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(payload: &[u8]) -> Vec<u8> {
        let mut r = vec![REPORT, 0xF9];
        r.extend(payload.iter().map(|b| !b));
        r.resize(65, 0xFF);
        r
    }

    #[test]
    fn request_is_inverted() {
        let r = request(65);
        assert_eq!(&r[..3], &[0x12, 0xF9, 0xFF]);
        assert!(r[2..].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn receiver_and_cable() {
        let rx = reply(&[0x53, 0x52, 0x20, 0x10, 1, 2, 3, 4, 0x09, 64, 0]);
        assert_eq!(
            parse(&rx, [VID, VID]),
            Some(Status::Awake {
                level: 64,
                charging: false,
                cable: false
            })
        );
        let usb = reply(&[0x53, 0x52, 0x20, 0x10, 1, 2, 3, 4, 0x00, 90, 1]);
        assert_eq!(
            parse(&usb, [VID, VID]),
            Some(Status::Awake {
                level: 90,
                charging: true,
                cable: true
            })
        );
        let realtek = reply(&[0xE4, 0x41, 0x0C, 0x10, 1, 2, 3, 4, 0x09, 50, 0]);
        assert!(parse(&realtek, [0x41E4, VID_REALTEK]).is_some());
        assert_eq!(parse(&realtek, [VID, VID]), None);
    }

    #[test]
    fn empty_echo_is_asleep() {
        assert_eq!(parse(&reply(&[0; 11]), [VID, VID]), Some(Status::Asleep));
    }
}
