use std::thread::sleep;
use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const RECEIVERS: &[(u16, &[u16])] = &[
    (0x0DB0, &[0x197E, 0x8E9A]),
    (0x145F, &[0x0376, 0x038D]),
    (0x14A5, &[0xC045]),
    (0x25AA, &[0x2008, 0x4008, 0x4018]),
    (
        0x3151,
        &[
            0x4011, 0x4014, 0x4017, 0x4021, 0x4027, 0x402D, 0x4038, 0x5006, 0x5007, 0x5008, 0x5020,
            0x5026, 0x502B, 0x5031, 0x5037, 0x5038, 0x503D, 0x503F, 0x5040, 0x5042, 0x5045, 0x504B,
            0x504E, 0x5050, 0x5052, 0x5055, 0x505A,
        ],
    ),
    (
        0x342D,
        &[
            0xE3E0, 0xE3E1, 0xE3E2, 0xE3E6, 0xE3E7, 0xE3E8, 0xE3F5, 0xE3F6, 0xE3F7, 0xE3FB, 0xE3FC,
            0xE3FD,
        ],
    ),
    (0x347A, &[0xA302, 0xA305]),
    (
        0x374A,
        &[
            0xA205, 0xA217, 0xA220, 0xA223, 0xA226, 0xA229, 0xA234, 0xA237, 0xA251, 0xA263, 0xA268,
            0xA311, 0xA314, 0xA319, 0xA322, 0xA329, 0xA355,
        ],
    ),
    (0x379A, &[0x1804, 0x1807]),
    (0x38A9, &[0x0002]),
    (0x38EE, &[0x0000, 0x0002, 0x0039]),
    (
        0x391D,
        &[
            0x1A04, 0x1A08, 0x1A0E, 0x1A13, 0x1A25, 0x1A2B, 0x1A34, 0x2017, 0x2A05,
        ],
    ),
];

/// Command bytes FoxRGB's lighting drivers use on the same feature report of some of these receivers.
const LIGHTING_ECHOES: [u8; 5] = [0x07, 0x87, 0x0E, 0x15, 0x08];

fn is_receiver(vid: u16, pid: u16) -> bool {
    RECEIVERS
        .iter()
        .any(|(v, pids)| *v == vid && pids.contains(&pid))
}

#[derive(Debug, PartialEq)]
struct Slots {
    keyboard: Option<u8>,
    mouse: Option<u8>,
}

/// `Some(level)` per connected slot; the level is 0 when unknown.
fn parse(r: &[u8]) -> Option<Slots> {
    if r.len() < 7 || r[2..7].iter().all(|&b| b == 0) || LIGHTING_ECHOES.contains(&r[1]) {
        return None;
    }
    let slot = |level: u8, offline: u8| (offline == 0).then_some(level);
    Some(Slots {
        keyboard: slot(r[2], r[4]),
        mouse: slot(r[3], r[5]),
    })
}

/// The product string without generic words: "MonsGeek 2.4G Wireless Keyboard" gives "MonsGeek".
fn brand(product: &str) -> String {
    const GENERIC: [&str; 13] = [
        "2.4G", "2.4GHZ", "8K", "4K", "2K", "1K", "RECEIVER", "WIRELESS", "DONGLE", "USB",
        "KEYBOARD", "MOUSE", "GAMING",
    ];
    product
        .split_whitespace()
        .filter(|w| !GENERIC.contains(&w.to_uppercase().as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn name(vid: u16, pid: u16, product: &str, keyboard: bool) -> String {
    let generic = product.is_empty() || product.contains("Receiver") || product.contains("2.4G");
    let brand = brand(product);
    let other_brand = !brand.is_empty() && !brand.to_uppercase().contains("ATTACK SHARK");
    let what = if keyboard { "keyboard" } else { "mouse" };
    match (vid, pid) {
        (0x3151, 0x402D) if !keyboard && !other_brand => "Attack Shark R3".into(),
        _ if generic && !brand.is_empty() => format!("{brand} {what}"),
        (0x3151, 0x5006) if keyboard && product.contains("Akko") => "Akko Air 01".into(),
        _ if generic => format!("Wireless {what}"),
        _ => product.to_string(),
    }
}

fn query(dev: &Dev) -> Option<Slots> {
    let mut req = vec![0u8; dev.coll.feat_len as usize];
    req[1] = 0xF7;
    for attempt in 0..3 {
        if attempt > 0 {
            sleep(Duration::from_millis(100));
        }
        if !dev.set_feature(&req) {
            return None;
        }
        sleep(Duration::from_millis(20));
        if let Some(s) = dev.get_feature(0).as_deref().and_then(parse) {
            return Some(s);
        }
    }
    None
}

pub fn poll_rongyuan(ctx: &mut Ctx) {
    let mut products: Vec<(u16, u16)> = ctx
        .colls
        .iter()
        .filter(|c| is_receiver(c.vid, c.pid) && !ctx.is_claimed(c.vid, c.pid))
        .map(|c| (c.vid, c.pid))
        .collect();
    products.sort();
    products.dedup();

    for (vid, pid) in products {
        let colls = ctx.find(vid, pid, |c: &Coll| {
            c.iface == 2 && c.page == 0xFFFF && c.usage == 0x0002 && c.feat_len >= 7
        });
        for c in colls {
            let Some(slots) = Dev::open_feature_only(&c).and_then(|d| query(&d)) else {
                continue;
            };
            for (keyboard, level) in [(true, slots.keyboard), (false, slots.mouse)] {
                let Some(level) = level else { continue };
                let tag = if keyboard { "kb" } else { "mouse" };
                let b = Battery {
                    key: ctx.stable_key(&format!("rongyuan-{}:{}-{tag}", hex4(vid), hex4(pid))),
                    name: name(vid, pid, &c.product, keyboard),
                    via: "2.4 GHz receiver".into(),
                    kind: if keyboard {
                        Kind::Keyboard
                    } else {
                        Kind::Mouse
                    },
                    percent: (1..=100).contains(&level).then_some(level),
                    // A sleeping keyboard drops off the receiver's list; keep its last level.
                    grace_secs: if keyboard { 1800 } else { 0 },
                    vid,
                    ..Default::default()
                };
                ctx.out.push(b);
            }
            ctx.claim(vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_slots() {
        let r = [0x00, 0x01, 80, 55, 0, 0, 0];
        assert_eq!(
            parse(&r),
            Some(Slots {
                keyboard: Some(80),
                mouse: Some(55)
            })
        );
        let r = [0x00, 0x01, 0, 70, 1, 0, 0];
        assert_eq!(
            parse(&r),
            Some(Slots {
                keyboard: None,
                mouse: Some(70)
            })
        );
    }

    #[test]
    fn rejects_empty_and_lighting_frames() {
        assert_eq!(parse(&[0, 1, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse(&[0, 0x87, 1, 3, 4, 0xFF, 0]), None);
        assert_eq!(parse(&[0, 1, 2]), None);
    }

    #[test]
    fn names_drop_link_words() {
        assert_eq!(brand("AJAZZ 2.4G 8K"), "AJAZZ");
        assert_eq!(brand("2.4G Wireless Receiver"), "");
        assert_eq!(
            name(0x3151, 0x402D, "2.4G Wireless Receiver", false),
            "Attack Shark R3"
        );
        assert_eq!(name(0x3151, 0x402D, "AJAZZ 2.4G 8K", false), "AJAZZ mouse");
        assert_eq!(
            name(0x3151, 0x5038, "MonsGeek 2.4G Wireless Keyboard", true),
            "MonsGeek keyboard"
        );
        assert_eq!(name(0x3151, 0x4011, "", true), "Wireless keyboard");
    }
}
