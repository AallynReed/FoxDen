use std::thread::sleep;
use std::time::Duration;

use super::vendor_frames::{gwolves_cable, gwolves_name};
use crate::hid::{Coll, Dev};
use crate::util::{guess_kind, kind_from_collections};
use crate::{Battery, Ctx, Kind};

struct Vendor {
    vid: u16,
    key: &'static str,
    /// How long the receiver needs before the reply can be fetched.
    delay_ms: u64,
    name: fn(u16) -> Option<&'static str>,
    cable: fn(u16) -> bool,
    only: Option<fn(&Coll) -> bool>,
}

fn glorious_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x2022 | 0x2011 => "Glorious Model O Wireless",
        0x2023 | 0x2012 => "Glorious Model D Wireless",
        0x2024 | 0x2013 => "Glorious Model O- Wireless",
        0x2025 | 0x2014 => "Glorious Model D- Wireless",
        0x2033 => "Glorious Model O 2 Wireless",
        0x2034 | 0x2036 | 0x201A => "Glorious Model D 2 PRO",
        _ => return None,
    })
}

fn glorious_cable(pid: u16) -> bool {
    matches!(pid, 0x2011..=0x2014 | 0x201A)
}

fn hitscan_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x0136 => "Hitscan Hyperlight X",
        0x0194 => "Hitscan Hyperlight N",
        _ => "Hitscan Hyperlight",
    })
}

const VENDORS: &[Vendor] = &[
    Vendor {
        vid: 0x258A,
        key: "glorious",
        delay_ms: 50,
        name: glorious_name,
        cable: glorious_cable,
        only: None,
    },
    Vendor {
        vid: 0x3770,
        key: "hitscan",
        delay_ms: 200,
        name: hitscan_name,
        cable: |pid| matches!(pid, 0x0136 | 0x0194),
        only: None,
    },
    Vendor {
        vid: 0x33E4,
        key: "gwolves",
        delay_ms: 100,
        name: |pid| Some(gwolves_name(pid)),
        cable: gwolves_cable,
        only: None,
    },
    Vendor {
        vid: 0x36A7,
        key: "wlmouse",
        delay_ms: 100,
        name: |_| Some("WLMouse YING"),
        cable: |_| false,
        only: Some(|c| c.pid == 0xA874 && c.page == 0xFFFF && c.usage == 0),
    },
];

/// Feature report 0, 65 bytes: `[3]=02 [4]=02 [6]=83`.
fn request() -> Vec<u8> {
    let mut r = vec![0u8; 65];
    r[3] = 0x02;
    r[4] = 0x02;
    r[6] = 0x83;
    r
}

#[derive(Debug, PartialEq)]
enum Status {
    Awake {
        pct: u8,
        charging: bool,
    },
    Waking,
    /// Asleep or out of reach; the charging flag may still be set.
    Away {
        charging: bool,
    },
}

/// Reply `[0]` report id, `[1]` status (A1 awake, A0 waking), `[6]` 83, `[7]` charging, `[8]` percent.
fn parse_reply(r: &[u8]) -> Option<Status> {
    if r.len() < 9 || r[6] != 0x83 || r[1] & 0xF0 != 0xA0 {
        return None;
    }
    let charging = r[7] == 0x01;
    Some(match r[1] {
        0xA1 if r[8] <= 100 => Status::Awake {
            pct: r[8].max(1),
            charging,
        },
        0xA0 => Status::Waking,
        _ => Status::Away { charging },
    })
}

fn ask(dev: &Dev, delay: Duration) -> Option<Status> {
    let mut status = None;
    for _ in 0..2 {
        if !dev.set_feature(&request()) {
            return None;
        }
        sleep(delay);
        status = Some(parse_reply(&dev.get_feature(0)?)?);
        if status != Some(Status::Waking) {
            break;
        }
        sleep(Duration::from_millis(400));
    }
    status
}

/// Products of other brands on the same chip: their own name, and keyboard or mouse from their collections.
fn sinowealth_kind(all: &[Coll], vid: u16, pid: u16, name: &str) -> Kind {
    let k = guess_kind(name);
    if k != Kind::Other {
        return k;
    }
    let keyboards = all
        .iter()
        .filter(|c| c.vid == vid && c.pid == pid && c.page == 0x01 && c.usage == 0x06)
        .count();
    if keyboards >= 2 {
        Kind::Keyboard
    } else {
        kind_from_collections(all, vid, pid)
    }
}

pub fn poll_glorious(ctx: &mut Ctx) {
    for v in VENDORS {
        for (vid, pid) in ctx.products(v.vid) {
            let colls = ctx.find(vid, pid, |c| {
                c.feat_len == 65 && v.only.is_none_or(|f| f(c))
            });
            for c in colls {
                let Some(status) = Dev::open_feature_only(&c)
                    .and_then(|d| ask(&d, Duration::from_millis(v.delay_ms)))
                else {
                    continue;
                };
                let cable = (v.cable)(pid);
                let (name, kind) = match (v.name)(pid) {
                    Some(n) => (n.to_string(), Kind::Mouse),
                    None => {
                        let n = if c.product.is_empty() {
                            "Wireless device".to_string()
                        } else {
                            c.product.clone()
                        };
                        let k = sinowealth_kind(&ctx.colls, vid, pid, &n);
                        (n, k)
                    }
                };
                let mut b = Battery {
                    key: ctx.stable_key(&format!("{}-{vid:04X}:{pid:04X}", v.key)),
                    name,
                    via: if cable {
                        "USB cable"
                    } else {
                        "2.4 GHz receiver"
                    }
                    .into(),
                    kind,
                    charging: cable,
                    ..Default::default()
                };
                match status {
                    Status::Awake { pct, charging } => {
                        b.percent = Some(pct);
                        b.charging |= charging;
                    }
                    Status::Away { charging } => {
                        b.asleep = true;
                        b.charging |= charging;
                    }
                    Status::Waking => b.asleep = true,
                }
                ctx.add(b, vid, pid);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u8, chg: u8, pct: u8) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        r[1] = status;
        r[4] = 0x02;
        r[6] = 0x83;
        r[7] = chg;
        r[8] = pct;
        r
    }

    #[test]
    fn status_byte() {
        assert_eq!(
            parse_reply(&reply(0xA1, 1, 57)),
            Some(Status::Awake {
                pct: 57,
                charging: true
            })
        );
        assert_eq!(
            parse_reply(&reply(0xA1, 0, 0)),
            Some(Status::Awake {
                pct: 1,
                charging: false
            })
        );
        assert_eq!(parse_reply(&reply(0xA0, 0, 0)), Some(Status::Waking));
        assert_eq!(
            parse_reply(&reply(0xA4, 0, 57)),
            Some(Status::Away { charging: false })
        );
        assert_eq!(parse_reply(&reply(0x00, 0, 57)), None);
        let mut lighting = reply(0xA1, 0, 57);
        lighting[6] = 0x02;
        assert_eq!(parse_reply(&lighting), None);
        assert_eq!(request()[..7], [0, 0, 0, 0x02, 0x02, 0, 0x83]);
    }

    #[test]
    fn model_tables() {
        assert_eq!(glorious_name(0x2011), Some("Glorious Model O Wireless"));
        assert_eq!(glorious_name(0x0049), None);
        assert!(glorious_cable(0x201A) && !glorious_cable(0x2022));
    }
}
