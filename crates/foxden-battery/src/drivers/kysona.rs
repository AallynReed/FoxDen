use std::time::Duration;

use super::vendor_frames::{gwolves_cable, gwolves_name, report8, report8_reply};
use crate::hid::{Coll, Dev};
use crate::util::kind_from_collections;
use crate::{Battery, Ctx, Kind};

const COMPX: u16 = 0x3554;
const ATK: u16 = 0x373B;
const HITSCAN: u16 = 0x3770;
const GWOLVES: u16 = 0x33E4;
const PULSAR: u16 = 0x3710;
const HECATE: u16 = 0x35BB;
const VENDORS: [u16; 6] = [COMPX, ATK, HITSCAN, GWOLVES, PULSAR, HECATE];

const ONLINE: u8 = 0x03;
const BATTERY: u8 = 0x04;

/// Products on these vendor ids that speak something else and must never get these frames.
fn excluded(vid: u16, pid: u16) -> bool {
    match vid {
        ATK => matches!(pid, 0x11CA | 0x11CB | 0x1203 | 0x1204 | 0x1178),
        COMPX => matches!(pid, 0xFA09 | 0xFB40),
        HECATE => !matches!(pid, 0xD3FA..=0xD3FD),
        _ => false,
    }
}

fn redragon(vid: u16, pid: u16) -> bool {
    vid == COMPX && matches!(pid, 0xF55D | 0xF5D5 | 0xF55E)
}

fn vxe_r1(vid: u16, pid: u16) -> bool {
    vid == COMPX && matches!(pid, 0xF58A | 0xF58C | 0xF58E | 0xF58F | 0xF505 | 0xF506)
}

/// A VXE R1 cable is listed under its receiver, so the mouse keeps one key.
fn key_pid(vid: u16, pid: u16) -> u16 {
    match (vid, pid) {
        (COMPX, 0xF58C) => 0xF58A,
        (COMPX, 0xF58F) => 0xF58E,
        (COMPX, 0xF506) => 0xF505,
        _ => pid,
    }
}

fn cable(vid: u16, pid: u16) -> bool {
    match vid {
        GWOLVES => gwolves_cable(pid),
        HECATE => matches!(pid, 0xD3FA | 0xD3FC | 0xD3FD),
        COMPX => matches!(pid, 0xF55E | 0xF58C | 0xF58F | 0xF506),
        _ => false,
    }
}

fn mouse_name(product: &str, vid: u16, pid: u16) -> String {
    if vid == GWOLVES {
        return gwolves_name(pid).into();
    }
    if redragon(vid, pid) {
        return "Redragon wireless mouse".into();
    }
    if vxe_r1(vid, pid) {
        return "VXE Dragonfly R1".into();
    }
    match vid {
        HITSCAN => return "Hitscan Hyperlight".into(),
        HECATE => return "HECATE mouse".into(),
        _ => {}
    }
    let mut n = product;
    for suffix in [" Dongle", " Receiver", " 2.4G", " Wireless Receiver"] {
        if n.len() > suffix.len()
            && let Some(head) = n.strip_suffix(suffix)
        {
            n = head;
        }
    }
    match (n.is_empty(), vid) {
        (true, ATK) => "ATK wireless mouse".into(),
        (true, _) => "Wireless mouse".into(),
        _ => n.into(),
    }
}

fn candidate(c: &Coll) -> bool {
    // FF60 is the QMK / VIA raw channel, which lighting uses.
    let vendor = c.page >= 0xFF00 && c.page != 0xFF60 && c.out_len >= 17 && c.in_len >= 17;
    (c.out_len == 17 && c.in_len == 17) || vendor
}

fn query(dev: &Dev, cmd: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&report8(cmd)) {
        return None;
    }
    dev.read_until(Duration::from_millis(500), |r| {
        report8_reply(r, cmd).then(|| r.to_vec())
    })
}

#[derive(Debug, Default, PartialEq)]
struct State {
    connected: bool,
    percent: Option<u8>,
    charging: bool,
    mv: u32,
    asleep: bool,
}

/// Online reply: `[6]` nonzero while the mouse is linked.
fn parse_online(r: &[u8]) -> bool {
    r[6] != 0
}

/// Battery reply: `[6]` percent, `[7]` charging, `[8..10]` millivolts big-endian.
fn parse_battery(r: &[u8]) -> State {
    State {
        connected: true,
        percent: (r[6] <= 100).then_some(r[6]),
        charging: r[7] != 0,
        mv: u16::from_be_bytes([r[8], r[9]]) as u32,
        asleep: false,
    }
}

/// `None` when nothing on this collection answers.
fn read_state(dev: &Dev) -> Option<State> {
    let online = query(dev, ONLINE).map(|r| parse_online(&r));
    if online == Some(false) {
        return Some(State::default());
    }
    match query(dev, BATTERY) {
        Some(r) => Some(parse_battery(&r)),
        // The receiver says linked but the mouse did not answer.
        None => online.map(|_| State {
            connected: true,
            asleep: true,
            ..State::default()
        }),
    }
}

pub fn poll_kysona(ctx: &mut Ctx) {
    let mut seen: Vec<String> = Vec::new();
    for vendor in VENDORS {
        for (vid, pid) in ctx.products(vendor) {
            if excluded(vid, pid) {
                continue;
            }
            let mut colls = ctx.find(vid, pid, candidate);
            colls.sort_by_key(|c| !(c.page == 0xFF02 && c.usage == 0x0002));
            for c in colls {
                let Some(state) = Dev::open(&c).and_then(|d| read_state(&d)) else {
                    continue;
                };
                if !state.connected {
                    ctx.claim(vid, pid);
                    break;
                }
                let name = mouse_name(&c.product, vid, pid);
                // A charging mouse can show up on its cable and its dongle at once.
                if seen.contains(&name) {
                    ctx.claim(vid, pid);
                    if state.charging {
                        ctx.out
                            .iter_mut()
                            .filter(|o| o.name == name)
                            .for_each(|o| o.charging = true);
                    }
                    break;
                }
                seen.push(name.clone());
                let kind = match kind_from_collections(&ctx.colls, vid, pid) {
                    Kind::Other => Kind::Mouse,
                    k => k,
                };
                let b = Battery {
                    key: ctx.stable_key(&format!("kysona-{vid:04X}:{:04X}", key_pid(vid, pid))),
                    name,
                    via: if cable(vid, pid) {
                        "USB cable"
                    } else {
                        "2.4 GHz receiver"
                    }
                    .into(),
                    kind,
                    percent: state.percent,
                    charging: state.charging,
                    asleep: state.asleep,
                    grace_secs: 1800,
                    ..Default::default()
                };
                ctx.add(b, vid, pid);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_and_online_replies() {
        let mut r = report8(BATTERY);
        r[6..10].copy_from_slice(&[64, 1, 0x0E, 0xD8]);
        assert_eq!(
            parse_battery(&r),
            State {
                connected: true,
                percent: Some(64),
                charging: true,
                mv: 3800,
                asleep: false
            }
        );
        let mut o = report8(ONLINE);
        assert!(!parse_online(&o));
        o[6] = 1;
        assert!(parse_online(&o));
    }

    #[test]
    fn names_and_links() {
        assert_eq!(
            mouse_name("Pulsar XS-1 8K Dongle", PULSAR, 0x5406),
            "Pulsar XS-1 8K"
        );
        assert_eq!(mouse_name("", ATK, 0x1234), "ATK wireless mouse");
        assert_eq!(
            mouse_name("VXE NordicMouse 1K Dongle", COMPX, 0xF58C),
            "VXE Dragonfly R1"
        );
        assert_eq!(mouse_name("x", GWOLVES, 0x3519), "G-Wolves Fenrir Asym");
        assert_eq!(key_pid(COMPX, 0xF58C), 0xF58A);
        assert!(cable(GWOLVES, 0x3519) && !cable(GWOLVES, 0x3854));
        assert!(excluded(COMPX, 0xFA09) && excluded(HECATE, 0xA009) && !excluded(HECATE, 0xD3FB));
    }
}
