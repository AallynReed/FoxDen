use std::thread::sleep;
use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3434;
const LINK_KM: u16 = 0xD026;
const NAPE_CABLE: u16 = 0x0440;
/// What the Link-KM lists in a slot for a paired Nape Pro.
const NAPE_IN_SLOT: u16 = 0x4004;
const LEGACY_RECEIVERS: [u16; 2] = [0xD030, 0xD031];
const ULTRA_LINK_8K: u16 = 0xD028;

const READ_WAIT: Duration = Duration::from_millis(600);

pub fn poll_keychron(ctx: &mut Ctx) {
    nape(ctx);
    legacy_receivers(ctx);
    ultra_link(ctx);
    for (vid, pid) in ctx.products(VID) {
        launcher_mouse(ctx, pid);
        link_keyboard(ctx, vid, pid);
    }
}

fn mouse(name: String, via: &str, percent: u8, charging: bool) -> Battery {
    Battery {
        name,
        via: via.into(),
        kind: Kind::Mouse,
        percent: Some(percent),
        charging,
        ..Default::default()
    }
}

fn clean_name(product: &str) -> String {
    if product.is_empty() || product.contains("Link") || product.contains("Receiver") {
        "Keychron mouse".into()
    } else {
        product.into()
    }
}

fn is_raw(c: &Coll) -> bool {
    c.page == 0xFF60 && c.usage == 0x61 && c.out_len == 33
}

/// Unnumbered 32-byte request on the raw HID channel; the reply echoes the request bytes.
fn raw_query(dev: &Dev, cmd: &[u8], min_len: usize, wait: Duration) -> Option<Vec<u8>> {
    let mut req = vec![0u8];
    req.extend_from_slice(cmd);
    dev.flush();
    if !dev.write(&req) {
        return None;
    }
    dev.read_until(wait, |r| {
        (r.len() >= min_len && r.starts_with(cmd)).then(|| r.to_vec())
    })
}

/// Link-KM B2 reply: three slots at 2/7/12 of `[vid BE][pid BE][status]`.
fn nape_linked(r: &[u8]) -> bool {
    r.len() >= 17
        && [2, 7, 12]
            .iter()
            .rev()
            .find(|&&o| u16::from_be_bytes([r[o + 2], r[o + 3]]) == NAPE_IN_SLOT)
            .is_some_and(|&o| r[o + 4] == 1)
}

/// A7 31 reply: `[A7][31][percent][charging]`.
fn parse_nape(r: &[u8]) -> Option<(u8, bool)> {
    (r.len() >= 4 && r[2] <= 100).then(|| (r[2], r[3] != 0))
}

fn nape(ctx: &mut Ctx) {
    for pid in [LINK_KM, NAPE_CABLE] {
        for c in ctx.find(VID, pid, is_raw) {
            let Some(dev) = Dev::open(&c) else { continue };
            if pid == LINK_KM
                && !raw_query(&dev, &[0xB2], 3, READ_WAIT).is_some_and(|r| nape_linked(&r))
            {
                continue;
            }
            let Some((pct, charging)) =
                raw_query(&dev, &[0xA7, 0x31], 4, READ_WAIT).and_then(|r| parse_nape(&r))
            else {
                continue;
            };
            let via = if pid == LINK_KM {
                "2.4 GHz receiver"
            } else {
                "USB cable"
            };
            let mut b = mouse("Keychron Nape Pro".into(), via, pct, charging);
            b.key = ctx.stable_key("keychron-nape-pro");
            ctx.add(b, VID, pid);
            return;
        }
    }
}

fn legacy_name(pid: u16) -> &'static str {
    match pid {
        0xD032 | 0xD033 => "Keychron M3",
        0xD035 => "Keychron M1",
        0xD036 | 0xD03E => "Keychron M3 mini",
        0xD03B => "Keychron M2",
        0xD03D => "Keychron M2 mini",
        0xD03F => "Keychron M6",
        0xD044 => "Keychron M7",
        _ => "Keychron mouse",
    }
}

/// Feature report 51, command 06: `(mouse pid, percent, charging)`.
fn parse_legacy(r: &[u8]) -> Option<(u16, u8, bool)> {
    if r.len() < 13 || r[..3] != [0x51, 0x06, 0x01] || r[4..6] != [0x34, 0x34] || r[11] > 100 {
        return None;
    }
    Some((u16::from_le_bytes([r[6], r[7]]), r[11], r[12] == 0x01))
}

/// 1K receivers: set feature 51 queues a status command, get feature 51 returns the answer.
fn legacy_receivers(ctx: &mut Ctx) {
    for rpid in LEGACY_RECEIVERS {
        for c in ctx.find(VID, rpid, |c| c.page == 0x8C && c.feat_len >= 16) {
            let Some(dev) = Dev::open(&c).or_else(|| Dev::open_feature_only(&c)) else {
                continue;
            };
            if !dev.set_feature(&[0x51, 0x06]) {
                continue;
            }
            let reply = (0..5).find_map(|_| {
                sleep(Duration::from_millis(40));
                dev.get_feature(0x51)
                    .filter(|r| r.len() >= 13 && r[..2] == [0x51, 0x06])
            });
            let Some((mpid, pct, charging)) = reply.as_deref().and_then(parse_legacy) else {
                continue;
            };
            let mut b = mouse(legacy_name(mpid).into(), "2.4 GHz receiver", pct, charging);
            b.key = ctx.stable_key(&format!("keychron-3434:{}", hex4(mpid)));
            ctx.add(b, VID, rpid);
            break;
        }
    }
}

/// Numbered report `[id][cmd]` on FFC1; the answer has `cmd` after its report id.
fn ultra_query(dev: &Dev, id: u8, cmd: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&[id, cmd]) {
        return None;
    }
    dev.read_until(READ_WAIT, |r| {
        (r.len() >= 21 && r[1] == cmd).then(|| r.to_vec())
    })
}

/// B5 03 reply: slots at 3/8/13 of `[vid LE][pid LE][status...]`.
fn ultra_linked(r: &[u8]) -> bool {
    [3, 8, 13]
        .iter()
        .any(|&o| r.len() > o + 4 && (r[o] | r[o + 1]) != 0 && r[o + 4] != 0)
}

/// Power byte of the B3 06 base info: bits 0-6 percent, bit 7 charging.
fn parse_ultra_power(r: &[u8]) -> Option<(u8, bool)> {
    let p = *r.get(20)?;
    let pct = p & 0x7F;
    (1..=100).contains(&pct).then_some((pct, p & 0x80 != 0))
}

fn ultra_link(ctx: &mut Ctx) {
    let colls = ctx.find(VID, ULTRA_LINK_8K, |c| {
        c.page == 0xFFC1 && c.usage == 0x01 && c.out_len >= 21 && c.in_len >= 21
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        if !ultra_query(&dev, 0xB5, 0x03).is_some_and(|r| ultra_linked(&r)) {
            continue;
        }
        let Some((pct, charging)) =
            ultra_query(&dev, 0xB3, 0x06).and_then(|r| parse_ultra_power(&r))
        else {
            continue;
        };
        let mut b = mouse(clean_name(&c.product), "2.4 GHz receiver", pct, charging);
        b.key = ctx.stable_key("keychron-3434:D028");
        ctx.add(b, VID, ULTRA_LINK_8K);
        return;
    }
}

/// 64-byte unnumbered `[cmd][00][81][sub]...[sum]` where byte 63 = 0xA1 - sum of bytes 0..63.
fn launcher_request(cmd: u8, sub: u8) -> [u8; 65] {
    let mut r = [0u8; 65];
    r[1] = cmd;
    r[3] = 0x81;
    r[4] = sub;
    let sum = r[1..64].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    r[64] = 0xA1u8.wrapping_sub(sum);
    r
}

fn launcher_query(dev: &Dev, cmd: u8, sub: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&launcher_request(cmd, sub)) {
        return None;
    }
    dev.read_until(READ_WAIT, |r| {
        (r.len() >= 13 && r[0] == cmd && r[3] == sub).then(|| r.to_vec())
    })
}

fn name_4k(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x0620 | 0x0622 => "Keychron M3 mini 4K",
        0x0621 => "Keychron M4 4K",
        0x0624 => "Keychron M6 4K",
        0x07A0 => "Keychron M3 4K",
        _ => return None,
    })
}

/// How the mouse answered `01 00 81 01` (and, when needed, the receiver form `41 00 81 01`).
#[derive(Debug, PartialEq)]
enum Launcher {
    /// Carries the mouse's own vendor id.
    Classic {
        percent: u8,
        charging: bool,
    },
    Receiver8k {
        percent: u8,
        charging: bool,
    },
    Cable8k {
        percent: u8,
        charging: bool,
    },
}

fn tag(r: &[u8]) -> u16 {
    u16::from_le_bytes([r[6], r[7]])
}

fn classic(r1: &[u8]) -> Option<Launcher> {
    (tag(r1) == VID && r1[11] <= 100).then(|| Launcher::Classic {
        percent: r1[11],
        charging: r1[10] != 0,
    })
}

fn receiver_8k(r: &[u8]) -> Option<Launcher> {
    (r[10] <= 3 && r[11] <= 100).then(|| Launcher::Receiver8k {
        percent: r[11],
        charging: r[10] == 1,
    })
}

/// Byte 5 is the highest polling-rate level, which only a real 8K reply fills in.
fn cable_8k(r1: &[u8]) -> Option<Launcher> {
    (r1[10] <= 3 && (1..=100).contains(&r1[11]) && (1..=8).contains(&r1[5])).then(|| {
        Launcher::Cable8k {
            percent: r1[11],
            charging: r1[10] == 1,
        }
    })
}

fn launcher_mouse(ctx: &mut Ctx, pid: u16) {
    let colls = ctx.find(VID, pid, |c| {
        c.page >= 0xFF00 && c.out_len == 65 && c.in_len == 65
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        let r1 = launcher_query(&dev, 0x01, 0x01);
        let is_4k = c.page == 0xFF0A
            && c.usage == 0x01
            && r1
                .as_ref()
                .is_some_and(|r| ![VID, 0x362D].contains(&tag(r)));
        if is_4k {
            // 4K Link: `[41][..][..][01][..][state][percent]`
            let Some(r) = launcher_query(&dev, 0x41, 0x01).filter(|r| (1..=100).contains(&r[6]))
            else {
                continue;
            };
            let name = launcher_query(&dev, 0x40, 0x00)
                .and_then(|st| name_4k(u16::from_le_bytes([st[4], st[5]])))
                .map_or_else(|| clean_name(&c.product), String::from);
            let mut b = mouse(name, "2.4 GHz receiver", r[6], r[5] != 0);
            b.key = ctx.stable_key(&format!("keychron-3434:{}", hex4(pid)));
            ctx.add(b, VID, pid);
            return;
        }
        let found = r1
            .as_deref()
            .and_then(classic)
            .or_else(|| {
                launcher_query(&dev, 0x41, 0x01)
                    .as_deref()
                    .and_then(receiver_8k)
            })
            .or_else(|| r1.as_deref().and_then(cable_8k));
        let (via, percent, charging) = match found {
            Some(
                Launcher::Classic { percent, charging }
                | Launcher::Receiver8k { percent, charging },
            ) => ("2.4 GHz receiver", percent, charging),
            Some(Launcher::Cable8k { percent, charging }) => ("USB cable", percent, charging),
            None => continue,
        };
        let mut b = mouse(clean_name(&c.product), via, percent, charging);
        b.key = ctx.stable_key(&format!("keychron-3434:{}", hex4(pid)));
        ctx.add(b, VID, pid);
        return;
    }
}

/// B2 from a Keychron Link: up to five slots of `[vid LE][pid LE][connected]`; the first linked Keychron.
fn linked_keyboard(r: &[u8]) -> Option<u16> {
    (0..r[1].min(5) as usize)
        .map(|i| 2 + 5 * i)
        .take_while(|&o| o + 4 < r.len())
        .find(|&o| u16::from_le_bytes([r[o], r[o + 1]]) == VID && r[o + 4] != 0)
        .map(|o| u16::from_le_bytes([r[o + 2], r[o + 3]]))
}

#[derive(Debug, PartialEq)]
struct KeyboardPower {
    percent: u8,
    millivolts: u32,
    charging: bool,
    full: bool,
}

/// A4 (battery patch firmware): `[A4][percent][mV LE][0 battery / 1 charging / 2 full]`.
fn parse_keyboard(r: &[u8]) -> Option<KeyboardPower> {
    (r.len() >= 5 && r[1] <= 100).then(|| KeyboardPower {
        percent: r[1],
        millivolts: u16::from_le_bytes([r[2], r[3]]) as u32,
        charging: r[4] == 1,
        full: r[4] == 2,
    })
}

/// Keyboards behind a Keychron Link, or on the cable; stock firmware never answers A4.
fn link_keyboard(ctx: &mut Ctx, vid: u16, pid: u16) {
    if pid == LINK_KM || pid == NAPE_CABLE {
        return;
    }
    let a4 = |dev: &Dev| raw_query(dev, &[0xA4], 8, Duration::from_millis(400));
    for c in ctx.find(vid, pid, is_raw) {
        let Some(dev) = Dev::open(&c) else { continue };
        let (kb_pid, reply, via) = match raw_query(&dev, &[0xB2], 8, Duration::from_millis(300)) {
            Some(r) => {
                let Some(kb) = linked_keyboard(&r) else {
                    return;
                };
                // The first request after an idle link is often lost while it wakes up.
                let Some(r) = a4(&dev).or_else(|| a4(&dev)) else {
                    return;
                };
                (kb, r, "2.4 GHz receiver")
            }
            None => match a4(&dev) {
                Some(r) => (pid, r, "USB cable"),
                None => continue,
            },
        };
        let Some(p) = parse_keyboard(&reply) else {
            return;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("keychron-kb-3434:{}", hex4(kb_pid))),
            name: if (0x0A50..=0x0A58).contains(&kb_pid) {
                "Keychron K5 Max"
            } else {
                "Keychron keyboard"
            }
            .into(),
            via: via.into(),
            kind: Kind::Keyboard,
            percent: Some(p.percent),
            millivolts: p.millivolts,
            charging: p.charging,
            full: p.full,
            ..Default::default()
        };
        ctx.add(b, vid, pid);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_checksum() {
        let r = launcher_request(0x41, 0x01);
        assert_eq!(&r[..5], &[0x00, 0x41, 0x00, 0x81, 0x01]);
        assert_eq!(r[1..].iter().fold(0u8, |s, &b| s.wrapping_add(b)), 0xA1);
    }

    #[test]
    fn launcher_reply_shapes() {
        let mut r = [0u8; 64];
        r[..4].copy_from_slice(&[0x01, 0, 0, 0x01]);
        r[6..8].copy_from_slice(&[0x34, 0x34]);
        r[10] = 1;
        r[11] = 72;
        assert_eq!(
            classic(&r),
            Some(Launcher::Classic {
                percent: 72,
                charging: true
            })
        );
        r[6] = 0x2D;
        r[5] = 4;
        assert_eq!(classic(&r), None);
        assert_eq!(
            cable_8k(&r),
            Some(Launcher::Cable8k {
                percent: 72,
                charging: true
            })
        );
        r[0] = 0x41;
        r[10] = 3;
        assert_eq!(
            receiver_8k(&r),
            Some(Launcher::Receiver8k {
                percent: 72,
                charging: false
            })
        );
    }

    #[test]
    fn legacy_status() {
        let r = [
            0x51, 0x06, 0x01, 0x00, 0x34, 0x34, 0x33, 0xD0, 0, 0, 0, 55, 0x01,
        ];
        let (pid, pct, chg) = parse_legacy(&r).unwrap();
        assert_eq!((legacy_name(pid), pct, chg), ("Keychron M3", 55, true));
    }

    #[test]
    fn receivers_and_slots() {
        let mut b2 = [0u8; 32];
        b2[..7].copy_from_slice(&[0xB2, 0x02, 0x34, 0x34, 0x52, 0x0A, 0x01]);
        assert_eq!(linked_keyboard(&b2), Some(0x0A52));
        b2[6] = 0;
        assert_eq!(linked_keyboard(&b2), None);

        let mut km = [0u8; 32];
        km[0] = 0xB2;
        km[7..12].copy_from_slice(&[0x34, 0x34, 0x40, 0x04, 0x01]);
        assert!(nape_linked(&km));
        assert_eq!(parse_nape(&[0xA7, 0x31, 81, 1]), Some((81, true)));

        let mut ultra = [0u8; 64];
        ultra[..2].copy_from_slice(&[0xB5, 0x03]);
        ultra[8..13].copy_from_slice(&[0x34, 0x34, 0x28, 0xD0, 0x01]);
        assert!(ultra_linked(&ultra));
        let mut info = [0u8; 64];
        info[20] = 0x80 | 64;
        assert_eq!(parse_ultra_power(&info), Some((64, true)));
    }

    #[test]
    fn keyboard_power() {
        let p = parse_keyboard(&[0xA4, 90, 0x10, 0x10, 2, 0, 0, 0]).unwrap();
        assert_eq!(
            p,
            KeyboardPower {
                percent: 90,
                millivolts: 4112,
                charging: false,
                full: true
            }
        );
    }
}
