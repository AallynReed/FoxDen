use std::time::Duration;

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x37D7;

struct Model {
    pid: u16,
    /// Output report id for commands, 0 when unnumbered.
    rid: u8,
    /// Model ids the info reply may carry in `[5]`.
    ids: &'static [u8],
    name: &'static str,
}

const MODELS: [Model; 2] = [
    Model {
        pid: 0x2401,
        rid: 0x00,
        ids: &[130],
        name: "Flydigi Vader 5 Pro",
    },
    Model {
        pid: 0x2501,
        rid: 0x03,
        ids: &[128, 129, 133, 134, 135],
        name: "Flydigi Apex 5",
    },
];

/// A `5A A5 <cmd>` reply, with or without a leftover input report id 04 in front.
fn reply_for(r: &[u8], cmd: u8) -> Option<Vec<u8>> {
    let r = if r.len() >= 4 && r[..3] == [0x04, 0x5A, 0xA5] {
        &r[1..]
    } else {
        r
    };
    (r.len() >= 5 && r[..3] == [0x5A, 0xA5, cmd]).then(|| r.to_vec())
}

#[derive(Debug, PartialEq)]
struct Status {
    percent: u8,
    charging: bool,
    receiver: bool,
}

/// Info reply: `[5]` model id, `[6]` connection, `[11]` state (high nibble) and level 0..5 (low nibble).
fn parse_info(r: &[u8], ids: &[u8], modern: bool) -> Option<Status> {
    if r.len() < 12 || !ids.contains(&r[5]) {
        return None;
    }
    let (level, state) = (r[11] & 0x0F, r[11] >> 4);
    if state > 2 || level > 5 {
        return None;
    }
    let link = r[6];
    let valid = if modern {
        link <= 1
    } else {
        matches!(link, 1 | 2)
    };
    if !valid {
        return None;
    }
    Some(Status {
        percent: if state == 2 { 100 } else { level * 20 },
        charging: state == 1,
        receiver: link == if modern { 1 } else { 2 },
    })
}

fn ask(d: &Dev, rid: u8, cmd: u8, arg: u8) -> Option<Vec<u8>> {
    if !d.write(&[rid, 0x5A, 0xA5, cmd, arg, 0]) {
        return None;
    }
    d.read_until(Duration::from_millis(800), |r| reply_for(r, cmd))
}

fn read(d: &Dev, m: &Model) -> Option<Status> {
    d.flush();
    let modern = ask(d, m.rid, 0x07, 0).is_some_and(|r| r[3] == 1 && r[4] == 0);
    let info = ask(d, m.rid, 0x01, 2)?;
    parse_info(&info, m.ids, modern)
}

pub fn poll_flydigi(ctx: &mut Ctx) {
    for m in &MODELS {
        if ctx.is_claimed(VID, m.pid) {
            continue;
        }
        let colls = ctx.find(VID, m.pid, |c| {
            c.page == 0xFFA0 && c.usage == 1 && c.in_len >= 12 && c.out_len >= 6
        });
        for c in colls {
            let Some(s) = Dev::open(&c).and_then(|d| read(&d, m)) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&format!("flydigi-{}:{}", hex4(VID), hex4(m.pid))),
                name: m.name.into(),
                via: if s.receiver {
                    "2.4 GHz receiver"
                } else {
                    "USB cable"
                }
                .into(),
                kind: Kind::Gamepad,
                percent: Some(s.percent),
                charging: s.charging,
                ..Battery::default()
            };
            ctx.add(b, VID, m.pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_with_or_without_report_id() {
        assert!(reply_for(&[0x04, 0x5A, 0xA5, 0x01, 0, 0], 0x01).is_some());
        assert!(reply_for(&[0x5A, 0xA5, 0x07, 1, 0], 0x07).is_some());
        assert!(reply_for(&[0x5A, 0xA5, 0x07, 1, 0], 0x01).is_none());
    }

    #[test]
    fn info_levels_and_links() {
        let mut r = [0u8; 16];
        r[..3].copy_from_slice(&[0x5A, 0xA5, 0x01]);
        r[5] = 135;
        r[6] = 1;
        r[11] = 0x14;
        let ids = MODELS[1].ids;
        assert_eq!(
            parse_info(&r, ids, true),
            Some(Status {
                percent: 80,
                charging: true,
                receiver: true
            })
        );
        assert_eq!(
            parse_info(&r, ids, false),
            Some(Status {
                percent: 80,
                charging: true,
                receiver: false
            })
        );
        r[11] = 0x23;
        assert_eq!(parse_info(&r, ids, true).unwrap().percent, 100);
        r[5] = 130;
        assert_eq!(parse_info(&r, ids, true), None);
        r[5] = 135;
        r[6] = 2;
        assert_eq!(parse_info(&r, ids, true), None);
    }
}
