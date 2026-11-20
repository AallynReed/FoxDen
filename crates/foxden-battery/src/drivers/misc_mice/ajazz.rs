use std::thread::sleep;
use std::time::Duration;

use super::{mouse, sum, via};
use crate::Ctx;
use crate::hid::Dev;

// --- AJ179 family: receiver vendor 249A, cable vendor 248A, interface 2 ---

const AJ179_RECEIVER: u16 = 0x249A;
const AJ179_CABLE: u16 = 0x248A;

/// Device info reply to command 10: [1] status, [4..8] model, [12] charging, [13] percent, [14] linked.
fn aj179_reply(r: &[u8]) -> bool {
    r.len() >= 32 && r[0] == 0x10 && r[1] == 0x00 && sum(&r[4..31]) == r[31]
}

/// "M179" style model id as "Ajazz AJ179".
fn aj179_name(r: &[u8]) -> String {
    let id: String = r[4..8]
        .iter()
        .take_while(|b| (0x20..0x7F).contains(*b))
        .map(|&b| b as char)
        .collect();
    match id.strip_prefix('M') {
        Some(rest) if id.len() == 4 => format!("Ajazz AJ{rest}"),
        _ => "Ajazz mouse".into(),
    }
}

pub(super) fn poll_aj179(ctx: &mut Ctx) {
    for vendor in [AJ179_RECEIVER, AJ179_CABLE] {
        for (vid, pid) in ctx.products(vendor) {
            for coll in ctx.find(vid, pid, |c| {
                c.iface == 2 && c.out_len == 33 && c.in_len == 33
            }) {
                let Some(dev) = Dev::open(&coll) else {
                    continue;
                };
                dev.flush();
                if !dev.write(&[0x00, 0x10]) {
                    continue;
                }
                let Some(r) = dev.read_until(Duration::from_millis(1500), |r| {
                    aj179_reply(r).then(|| r.to_vec())
                }) else {
                    continue;
                };
                if r[13] > 100 {
                    continue;
                }
                let receiver = vid == AJ179_RECEIVER;
                let mut b = mouse(
                    ctx.stable_key(&format!("ajazz-{vid:04X}:{pid:04X}")),
                    &aj179_name(&r),
                    via(!receiver),
                );
                b.connected = !receiver || r[14] != 0;
                b.charging = r[12] != 0;
                b.percent = b.connected.then_some(r[13]);
                ctx.add(b, vid, pid);
                break;
            }
        }
    }
}

// --- AJ-series dongles on the SONiX vendor id 3151 ---

const SONIX: u16 = 0x3151;
/// Keyboards FoxDen lights on this vendor id: never sent the mouse status poll.
const LIGHTING_PIDS: [u16; 8] = [
    0x4002, 0x4003, 0x4010, 0x4011, 0x4015, 0x5026, 0x5029, 0x502D,
];

/// Feature report 05 after the F7 status poll: byte 3 is the charge; non-zero bytes 1..3 mean a reconnect frame.
fn aj_series_level(r: &[u8]) -> Option<u8> {
    (r.len() >= 4 && r[0] == 0x05 && r[1] == 0 && r[2] == 0 && r[3] <= 100).then_some(r[3])
}

pub(super) fn poll_aj_series(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(SONIX) {
        if LIGHTING_PIDS.contains(&pid) {
            continue;
        }
        for coll in ctx.find(vid, pid, |c| {
            c.page == 0xFFFF && c.usage == 0x02 && c.feat_len >= 65
        }) {
            let Some(dev) = Dev::open_feature_only(&coll) else {
                continue;
            };
            let mut req = vec![0u8; coll.feat_len as usize];
            req[1] = 0xF7;
            if !dev.set_feature(&req) {
                continue;
            }
            sleep(Duration::from_millis(30));
            let Some(pct) = dev.get_feature(0x05).as_deref().and_then(aj_series_level) else {
                continue;
            };
            let name = if coll.product.is_empty() {
                "AJAZZ mouse"
            } else {
                coll.product.as_str()
            };
            let mut b = mouse(
                ctx.stable_key(&format!("ajazz-aj-{vid:04X}:{pid:04X}")),
                name,
                via(false),
            );
            // 0: the link is up but the mouse is off.
            b.connected = pct > 0;
            b.percent = b.connected.then_some(pct);
            ctx.add(b, vid, pid);
            break;
        }
    }
}

// --- AJ139 V2 MC and siblings on PID 2255 (vendor A8A5 dongle, A8A4 cable) ---

const AJ139_PID: u16 = 0x2255;
const AJ139_DONGLE: u16 = 0xA8A5;
const AJ139_CABLE: u16 = 0xA8A4;

/// Reply AA 30: byte 8 percent, byte 9 non-zero on USB power.
fn aj139_status(r: &[u8]) -> Option<(u8, bool)> {
    (r.len() > 9 && r[0] == 0xAA && r[1] == 0x30).then(|| (r[8], r[9] != 0))
}

pub(super) fn poll_aj139(ctx: &mut Ctx) {
    // Two published variants of the request checksum byte.
    let requests: [(&[u8], u64); 2] = [
        (&[0x00, 0x55, 0x30, 0xA5, 0x2E, 0x2E], 700),
        (&[0x00, 0x55, 0x30, 0xA5, 0x0B, 0x2E, 0x01, 0x01], 800),
    ];
    for vendor in [AJ139_DONGLE, AJ139_CABLE] {
        if ctx.is_claimed(vendor, AJ139_PID) {
            continue;
        }
        for coll in ctx.find(vendor, AJ139_PID, |c| {
            c.page == 0xFF01 && c.usage == 0x10 && c.out_len >= 65 && c.in_len >= 65
        }) {
            let Some(dev) = Dev::open(&coll) else {
                continue;
            };
            dev.flush();
            let status = requests.iter().find_map(|&(req, ms)| {
                dev.write(req)
                    .then(|| dev.read_until(Duration::from_millis(ms), aj139_status))
                    .flatten()
            });
            let Some((pct, charging)) = status.filter(|s| s.0 <= 100) else {
                continue;
            };
            let cable = vendor == AJ139_CABLE;
            let mut b = mouse(
                ctx.stable_key(&format!("ajazz-aj139-{vendor:04X}:{AJ139_PID:04X}")),
                "AJAZZ wireless mouse",
                via(cable),
            );
            b.charging = charging;
            b.connected = cable || pct > 0;
            b.percent = b.connected.then_some(pct);
            ctx.add(b, vendor, AJ139_PID);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aj179_info() {
        let mut r = vec![0u8; 32];
        r[0] = 0x10;
        r[4..8].copy_from_slice(b"M179");
        r[12] = 1;
        r[13] = 55;
        r[14] = 1;
        r[31] = sum(&r[4..31]);
        assert!(aj179_reply(&r));
        assert_eq!(aj179_name(&r), "Ajazz AJ179");
        r[31] ^= 1;
        assert!(!aj179_reply(&r));
        r[4] = b'X';
        assert_eq!(aj179_name(&r), "Ajazz mouse");
    }

    #[test]
    fn aj_series_rejects_reconnect_frames() {
        assert_eq!(aj_series_level(&[0x05, 0, 0, 72]), Some(72));
        assert_eq!(aj_series_level(&[0x05, 1, 0, 72]), None);
        assert_eq!(aj_series_level(&[0x05, 0, 0, 101]), None);
    }

    #[test]
    fn aj139_status_frame() {
        let mut r = vec![0u8; 64];
        r[..2].copy_from_slice(&[0xAA, 0x30]);
        r[8] = 90;
        r[9] = 1;
        assert_eq!(aj139_status(&r), Some((90, true)));
        r[1] = 0xFA;
        assert_eq!(aj139_status(&r), None);
    }
}
