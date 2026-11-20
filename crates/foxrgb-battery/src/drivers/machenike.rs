use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x248A;
const RECEIVER: u16 = 0xFF4C;
const CABLE: u16 = 0xFF4E;
const REPORT: u8 = 0x51;
const STATUS_LEN: u16 = 21;

fn status_collection(c: &Coll, cable: bool) -> bool {
    c.iface == if cable { 2 } else { 1 }
        && c.page == 0x008C
        && c.usage == 1
        && c.in_len == 33
        && c.out_len == 33
        && c.feat_len == 1001
}

#[derive(Debug, PartialEq)]
struct Status {
    connected: bool,
    percent: Option<u8>,
    charging: bool,
}

/// Report 51 command 06: `[4..8]` the L7's ids, `[10]` link (low bits mode, bit 3 linked), `[11]` %, `[12]` charging.
fn parse_status(r: &[u8], cable: bool) -> Option<Status> {
    if r.len() != STATUS_LEN as usize
        || r[0] != REPORT
        || r[1] != 0x06
        || r[4..8] != [0x8A, 0x24, 0x4E, 0xFF]
    {
        return None;
    }
    if r[10] & 7 != u8::from(!cable) {
        return None;
    }
    // The link bit stays clear on the cable.
    let connected = cable || r[10] & 8 != 0;
    if connected && (r[11] > 100 || r[12] > 1) {
        return None;
    }
    Some(Status {
        connected,
        percent: connected.then_some(r[11]),
        charging: connected && r[12] == 1,
    })
}

fn query(dev: &Dev, cable: bool) -> Option<Status> {
    let mut req = vec![0u8; STATUS_LEN as usize];
    req[0] = REPORT;
    req[1] = 0x06;
    let end = Instant::now() + Duration::from_millis(200);
    if !dev.set_feature(&req) {
        return None;
    }
    loop {
        sleep(Duration::from_millis(10));
        if let Some(s) = dev
            .get_feature(REPORT)
            .and_then(|r| parse_status(&r, cable))
        {
            return Some(s);
        }
        if Instant::now() >= end {
            return None;
        }
    }
}

pub fn poll_machenike(ctx: &mut Ctx) {
    let mut have_cable = false;
    for pid in [CABLE, RECEIVER] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let cable = pid == CABLE;
        for mut c in ctx.find(VID, pid, |c| status_collection(c, cable)) {
            if c.product != "L7 Mix" {
                continue;
            }
            // Claimed even when the read fails: other 248A drivers must not write to it.
            ctx.claim(VID, pid);
            if have_cable {
                break;
            }
            c.feat_len = STATUS_LEN;
            let Some(s) = Dev::open_feature_only(&c).and_then(|d| query(&d, cable)) else {
                break;
            };
            let b = Battery {
                key: ctx.stable_key("machenike-l7"),
                name: "Machenike L7 Arctic / L7 Mix".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                percent: s.percent,
                charging: s.charging,
                connected: s.connected,
                grace_secs: if cable { 0 } else { 1800 },
                ..Default::default()
            };
            have_cable = cable;
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(link: u8, pct: u8, chg: u8) -> Vec<u8> {
        let mut r = vec![0u8; 21];
        r[..8].copy_from_slice(&[0x51, 0x06, 0, 0, 0x8A, 0x24, 0x4E, 0xFF]);
        r[10] = link;
        r[11] = pct;
        r[12] = chg;
        r
    }

    #[test]
    fn receiver_and_cable_status() {
        assert_eq!(
            parse_status(&frame(0x09, 64, 0), false),
            Some(Status {
                connected: true,
                percent: Some(64),
                charging: false
            })
        );
        assert_eq!(
            parse_status(&frame(0x01, 0xFF, 0xFF), false),
            Some(Status {
                connected: false,
                percent: None,
                charging: false
            })
        );
        assert_eq!(
            parse_status(&frame(0x00, 80, 1), true),
            Some(Status {
                connected: true,
                percent: Some(80),
                charging: true
            })
        );
        assert_eq!(parse_status(&frame(0x00, 80, 1), false), None);
        assert_eq!(parse_status(&frame(0x09, 101, 0), false), None);
    }
}
