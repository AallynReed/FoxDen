use std::time::Duration;

use crate::hid::Dev;
use crate::util::{clamp100, guess_kind, hex4, kind_from_collections};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1B1C;

const VOID_PIDS: &[u16] = &[
    0x1B1C, 0x1B27, 0x0A14, 0x0A16, 0x0A17, 0x0A1D, 0x0A1A, 0x1B2A, 0x1B23, 0x1B29, 0x0A55, 0x0A51,
    0x0A52, 0x0A38, 0x0A4F, 0x0A2B, 0x0A75, 0x0A56,
];
const BRAGI_PIDS: &[u16] = &[
    0x2A08, 0x2A02, 0x0A97, 0x0A64, 0x0A62, 0x0A3E, 0x0A3D, 0x2B00,
];
const COMPX_MICE: &[(u16, &str)] = &[
    (0x2B2A, "Corsair Sabre V2 Pro Ultralight"),
    (0x2B34, "Corsair Sabre v2 PRO WIRELESS MG"),
];
const PAIRED: &[(u32, &str, Kind)] = &[(0x1B7E, "Corsair Dark Core RGB Pro SE", Kind::Mouse)];

fn key(pid: u16, suffix: &str) -> String {
    format!("corsair-{}:{}{suffix}", hex4(VID), hex4(pid))
}

/// Void family status report: `[2]` level (bit 7 = mic up), `[4]` 0 off, 1-2 on battery, 4-5 charging.
fn parse_void(r: &[u8]) -> Option<(bool, Option<u8>, bool)> {
    if r.len() < 5 || r[4] > 5 {
        return None;
    }
    let st = r[4];
    let charging = matches!(st, 4 | 5);
    let percent = (matches!(st, 1 | 2) || charging).then(|| clamp100((r[2] & 0x7F) as i32));
    Some((st != 0, percent, charging))
}

fn poll_void(ctx: &mut Ctx, pid: u16) {
    for c in ctx.find(VID, pid, |c| {
        matches!(c.page, 0xFFC5 | 0xFF00) && c.out_len > 0
    }) {
        let Some(d) = Dev::open(&c) else { continue };
        d.flush();
        if !d.write(&[0xC9, 0x64]) {
            continue;
        }
        let Some((connected, percent, charging)) = d
            .read_until(Duration::from_millis(1000), |r| {
                (r.len() >= 5).then(|| parse_void(r))
            })
            .flatten()
        else {
            continue;
        };
        let name = if c.product.is_empty() {
            "Corsair wireless headset".into()
        } else {
            c.product.clone()
        };
        let b = Battery {
            key: ctx.stable_key(&key(pid, "")),
            name,
            via: "2.4 GHz receiver".into(),
            kind: Kind::Headset,
            percent,
            charging,
            connected,
            ..Battery::default()
        };
        ctx.add(b, VID, pid);
        return;
    }
}

fn compx_sum(b: &[u8]) -> u8 {
    0x55u8.wrapping_sub(b[..16].iter().fold(0u8, |s, &x| s.wrapping_add(x)))
}

fn compx_request(cmd: u8) -> [u8; 17] {
    let mut req = [0u8; 17];
    req[0] = 0x08;
    req[1] = cmd;
    req[16] = compx_sum(&req);
    req
}

fn compx_is_reply(r: &[u8], cmd: u8) -> bool {
    r.len() >= 17 && r[0] == 0x08 && r[1] == cmd && r[16] == compx_sum(r)
}

fn compx_query(d: &Dev, cmd: u8) -> Option<Vec<u8>> {
    d.flush();
    if !d.write(&compx_request(cmd)) {
        return None;
    }
    d.read_until(Duration::from_millis(500), |r| {
        compx_is_reply(r, cmd).then(|| r.to_vec())
    })
}

/// Battery reply: `[6]` percent, `[7]` charging, `[8..10]` mV big endian.
fn parse_compx_battery(r: &[u8]) -> (Option<u8>, bool, u32) {
    (
        (r[6] <= 100).then_some(r[6]),
        r[7] != 0,
        (r[8] as u32) << 8 | r[9] as u32,
    )
}

fn poll_compx(ctx: &mut Ctx, pid: u16, name: &str) {
    let colls = ctx.find(VID, pid, |c| {
        c.page == 0xFF02 && c.usage == 0x0002 && c.out_len >= 17 && c.in_len >= 17
    });
    for c in colls {
        let Some(d) = Dev::open(&c) else { continue };
        let link = compx_query(&d, 0x03).map(|r| r[6] != 0);
        let battery = if link != Some(false) {
            compx_query(&d, 0x04)
        } else {
            None
        };
        if link.is_none() && battery.is_none() {
            continue;
        }
        if battery.is_none() && link == Some(false) {
            ctx.claim(VID, pid);
            return;
        }
        let mut b = Battery {
            key: ctx.stable_key(&key(pid, "")),
            name: name.into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            grace_secs: 1800,
            ..Battery::default()
        };
        match battery {
            Some(r) => (b.percent, b.charging, b.millivolts) = parse_compx_battery(&r),
            None => b.asleep = true,
        }
        ctx.add(b, VID, pid);
        return;
    }
}

enum Get {
    Value(u32),
    Failed,
}

/// A Bragi GET reply, or `None` for notifications and replies meant for another sender.
fn parse_bragi(r: &[u8], short: bool, target: u8) -> Option<Get> {
    let o = usize::from(!short);
    let source = if target == 0x08 { 0x00 } else { 0x01 };
    if r.len() < o + 7 || r[o] != source || r[o + 1] != 0x02 {
        return None;
    }
    if r[o + 2] != 0 {
        return Some(Get::Failed);
    }
    Some(Get::Value(u32::from_le_bytes([
        r[o + 3],
        r[o + 4],
        r[o + 5],
        r[o + 6],
    ])))
}

/// One open Bragi channel. Older firmware puts a 02 before the target, newer receivers don't;
/// `short` holds whichever answered first on this handle.
struct Channel {
    dev: Dev,
    numbered: bool,
    short: Option<bool>,
}

impl Channel {
    fn get(&mut self, target: u8, prop: u8) -> Option<u32> {
        let tries: &[bool] = match self.short {
            Some(true) => &[true],
            Some(false) => &[false],
            None => &[false, true],
        };
        for &short in tries {
            if let Some(v) = self.send(short, target, prop) {
                self.short = Some(short);
                return Some(v);
            }
        }
        None
    }

    fn send(&self, short: bool, target: u8, prop: u8) -> Option<u32> {
        let req: &[u8] = match (short, self.numbered) {
            (true, _) => &[0x00, target, 0x02, prop],
            (false, true) => &[0x02, target, 0x02, prop],
            (false, false) => &[0x00, 0x02, target, 0x02, prop],
        };
        let _turn = foxrgb_core::hid_turn::take();
        self.dev.flush();
        if !self.dev.write(req) {
            return None;
        }
        match self.dev.read_until(Duration::from_millis(300), |r| {
            parse_bragi(r, short, target)
        })? {
            Get::Value(v) => Some(v),
            Get::Failed => None,
        }
    }
}

fn poll_bragi(ctx: &mut Ctx, pid: u16) {
    let known = BRAGI_PIDS.contains(&pid);
    let colls = ctx.find(VID, pid, |c| {
        let sizes = matches!(c.out_len, 64 | 65) && matches!(c.in_len, 64 | 65);
        sizes && (c.page == 0xFF42 || (known && (c.page >= 0xFF00 || c.iface == 4)))
    });
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        let mut bragi = Channel {
            dev,
            numbered: c.out_len == 64,
            short: None,
        };
        let mut any = false;
        for target in [0x09u8, 0x0A, 0x0B, 0x08] {
            if target == 0x08 && any {
                break;
            }
            let mut level = bragi.get(target, 0x0F);
            for _ in 0..2 {
                if !matches!(level, Some(0) | Some(1001..)) {
                    break;
                }
                level = bragi.get(target, 0x0F);
            }
            let Some(level @ 0..=1000) = level else {
                continue;
            };
            let mut name = if c.product.is_empty() {
                "Corsair wireless device".to_string()
            } else {
                c.product.clone()
            };
            let mut kind = guess_kind(&name);
            if target != 0x08
                && let Some(dev_pid) = bragi.get(target, 0x12)
            {
                if let Some(&(_, n, k)) = PAIRED.iter().find(|m| m.0 == dev_pid) {
                    name = n.into();
                    kind = k;
                } else if dev_pid >> 8 == 0x0A {
                    kind = Kind::Headset;
                    if c.product.contains("Receiver") {
                        name = "Corsair wireless headset".into();
                    }
                }
            }
            if kind == Kind::Other && pid >> 8 == 0x0A {
                kind = Kind::Headset;
            }
            if kind == Kind::Other {
                kind = kind_from_collections(&ctx.colls, VID, pid);
            }
            let charge = bragi.get(target, 0x10);
            let b = Battery {
                key: ctx.stable_key(&key(pid, &format!("-{target:02X}"))),
                name,
                via: if target == 0x08 {
                    "USB / Bluetooth"
                } else {
                    "Corsair receiver"
                }
                .into(),
                kind,
                percent: Some(clamp100((level as i32 + 5) / 10)),
                charging: matches!(charge, Some(1 | 3)),
                full: charge == Some(3),
                vid: VID,
                ..Battery::default()
            };
            ctx.out.push(b);
            any = true;
        }
        if any {
            ctx.claim(VID, pid);
            return;
        }
    }
}

pub fn poll_corsair(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        if VOID_PIDS.contains(&pid) {
            poll_void(ctx, pid);
        } else if let Some(&(_, name)) = COMPX_MICE.iter().find(|m| m.0 == pid) {
            poll_compx(ctx, pid, name);
        } else {
            poll_bragi(ctx, pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn void_status_byte_decides_state() {
        assert_eq!(
            parse_void(&[0x64, 0, 0x80 | 55, 0, 1]),
            Some((true, Some(55), false))
        );
        assert_eq!(
            parse_void(&[0x64, 0, 40, 0, 5]),
            Some((true, Some(40), true))
        );
        assert_eq!(parse_void(&[0x64, 0, 40, 0, 0]), Some((false, None, false)));
        assert_eq!(parse_void(&[0x64, 0, 40, 0, 9]), None);
    }

    #[test]
    fn compx_frames_carry_checksum() {
        let req = compx_request(0x04);
        assert_eq!(req[16], 0x55u8.wrapping_sub(0x0C));
        let mut r = [0u8; 17];
        r[..10].copy_from_slice(&[0x08, 0x04, 0, 0, 0, 0, 77, 1, 0x0F, 0xA0]);
        r[16] = compx_sum(&r);
        assert!(compx_is_reply(&r, 0x04));
        assert!(!compx_is_reply(&r, 0x03));
        assert_eq!(parse_compx_battery(&r), (Some(77), true, 4000));
    }

    #[test]
    fn bragi_replies_match_source_and_framing() {
        let prefixed = [0x02, 0x01, 0x02, 0x00, 0xE8, 0x03, 0, 0];
        assert!(matches!(
            parse_bragi(&prefixed, false, 0x09),
            Some(Get::Value(1000))
        ));
        assert!(parse_bragi(&prefixed, false, 0x08).is_none());
        let plain = [0x00, 0x02, 0x05, 0, 0, 0, 0];
        assert!(matches!(parse_bragi(&plain, true, 0x08), Some(Get::Failed)));
        assert!(parse_bragi(&[0x00, 0x01, 0, 0, 0, 0, 0], true, 0x08).is_none());
    }
}
