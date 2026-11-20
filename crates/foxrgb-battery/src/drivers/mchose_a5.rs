use std::collections::BTreeSet;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x2023;
const ROUTE_MOUSE: u8 = 0x02;
const PAGE: u8 = 0x00;
const BATTERY: u8 = 0x83;
const OK: u8 = 0xA1;

/// (pid, name, on its cable)
const MODELS: [(u16, &str, bool); 5] = [
    (0xF013, "MCHOSE A5 Pro / Pro Max", false),
    (0xF015, "MCHOSE A5 Pro / Pro Max", false),
    (0xF017, "MCHOSE A5 Pro", true),
    (0xF018, "MCHOSE A5 Pro Max", false),
    (0xF019, "MCHOSE A5 Pro Max", true),
];

/// Receivers whose mouse has answered at least once, so a silent one can show as asleep.
static ANSWERED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

fn request(len: usize) -> Vec<u8> {
    let mut req = vec![0u8; len.max(7)];
    req[3] = ROUTE_MOUSE;
    req[4] = 2;
    req[5] = PAGE;
    req[6] = BATTERY;
    req
}

/// (level, charging); the OK marker sits right after the report id, or first when it was dropped.
fn parse(r: &[u8]) -> Option<(u8, bool)> {
    let at = match r {
        [_, OK, ..] => 1,
        [OK, ..] => 0,
        _ => return None,
    };
    let f = r.get(at..at + 8)?;
    if f[4] != PAGE || f[5] != BATTERY || f[7] == 0xFF {
        return None;
    }
    Some((f[7].min(100), f[6] != 0))
}

enum Query {
    Failed,
    Silent,
    Level(u8, bool),
}

fn query(c: &Coll) -> Query {
    let Some(dev) = Dev::open_feature_only(c) else {
        return Query::Failed;
    };
    let req = request(c.feat_len as usize);
    if !dev.set_feature(&req) {
        return Query::Failed;
    }
    sleep(Duration::from_millis(34));
    for attempt in 0..8 {
        if attempt == 3 {
            if !dev.set_feature(&req) {
                return Query::Failed;
            }
            sleep(Duration::from_millis(34));
        }
        if let Some((level, charging)) = dev.get_feature(0).as_deref().and_then(parse) {
            return Query::Level(level, charging);
        }
        sleep(Duration::from_millis(30));
    }
    Query::Silent
}

pub fn poll_mchose_a5(ctx: &mut Ctx) {
    let mut answered = ANSWERED.lock().unwrap_or_else(|e| e.into_inner());
    for (pid, name, cable) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let mut colls = ctx.find(VID, pid, |c| c.page == 0xFFFF && c.feat_len >= 65);
        colls.sort_by_key(|c| c.iface != 2);
        let mut silent = false;
        let mut level = None;
        for c in &colls {
            match query(c) {
                Query::Level(l, ch) => {
                    level = Some((l, ch));
                    break;
                }
                Query::Silent => silent = true,
                Query::Failed => {}
            }
        }
        let mut b = Battery {
            key: ctx.stable_key(&format!("mchose-a5-{}", hex4(pid))),
            name: name.into(),
            via: if cable {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Mouse,
            ..Default::default()
        };
        match level {
            Some((l, ch)) => {
                answered.insert(pid);
                b.percent = Some(l);
                b.charging = ch;
            }
            None if silent && !cable && answered.contains(&pid) => b.asleep = true,
            None => continue,
        }
        ctx.add(b, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_request() {
        assert_eq!(
            &request(65)[..8],
            &[0x00, 0x00, 0x00, 0x02, 0x02, 0x00, 0x83, 0x00]
        );
    }

    #[test]
    fn battery_reply() {
        assert_eq!(
            parse(&[0x00, 0xA1, 0, 0, 0, 0x00, 0x83, 1, 85, 0]),
            Some((85, true))
        );
        assert_eq!(
            parse(&[0xA1, 0, 0, 0, 0x00, 0x83, 0, 120, 0]),
            Some((100, false))
        );
        assert_eq!(parse(&[0x00, 0xA3, 0, 0, 0, 0x00, 0x83, 0, 50, 0]), None);
        assert_eq!(parse(&[0x00, 0xA1, 0, 0, 0, 0x00, 0x83, 0, 0xFF, 0]), None);
    }
}
