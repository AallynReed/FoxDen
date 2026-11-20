use std::thread::sleep;
use std::time::Duration;

use super::oem_frames::query_status15;
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const SORA_VID: u16 = 0x1915;
const SORA_RECEIVER: u16 = 0xAE1C;
const SORA_CABLE: u16 = 0xAE11;

const TEN_VID: u16 = 0x093A;
const TEN_RECEIVER: u16 = 0xEB01;
const TEN_CABLE: u16 = 0xE020;

pub fn poll_ninjutso(ctx: &mut Ctx) {
    sora_v2(ctx);
    ten(ctx);
}

/// Sora V2 reports 0 for a full battery.
fn sora_percent(raw: u8) -> u8 {
    if raw == 0 { 100 } else { raw }
}

fn sora_v2(ctx: &mut Ctx) {
    for pid in [SORA_RECEIVER, SORA_CABLE] {
        if ctx.is_claimed(SORA_VID, pid) {
            continue;
        }
        let cable = pid == SORA_CABLE;
        for c in ctx.find(SORA_VID, pid, |c| {
            c.page == 0xFFA0 && c.usage == 1 && c.feat_len >= 32
        }) {
            let Some(dev) = Dev::open_feature_only(&c) else {
                continue;
            };
            let Some(s) = query_status15(&dev, cable) else {
                continue;
            };
            // The cable answers with online = 0.
            let connected = cable || s.online;
            let b = Battery {
                key: ctx.stable_key(&format!("ninjutso-1915:{pid:04X}")),
                name: "Ninjutso Sora V2".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                connected,
                percent: connected.then(|| sora_percent(s.percent)),
                charging: connected && s.charging,
                ..Default::default()
            };
            ctx.add(b, SORA_VID, pid);
            break;
        }
    }
}

/// Feature report 06: 13 = online, 12 = battery %, 11 = charging; the value is at [8].
fn ten_value(r: &[u8], cmd: u8) -> Option<u8> {
    (r.len() == 16 && r[0] == 0x06 && r[1] == cmd).then_some(r[8])
}

fn ten_read(dev: &Dev, cmd: u8) -> Option<u8> {
    let mut q = [0u8; 16];
    q[0] = 0x06;
    q[1] = cmd;
    q[4] = 1;
    if !dev.set_feature(&q) {
        return None;
    }
    (0..3).find_map(|_| {
        sleep(Duration::from_millis(30));
        dev.get_feature(0x06).and_then(|r| ten_value(&r, cmd))
    })
}

fn ten(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(TEN_VID) {
        if pid != TEN_RECEIVER && pid != TEN_CABLE {
            continue;
        }
        for c in ctx.find(vid, pid, |c| {
            c.page == 0xFF01 && c.usage == 1 && c.feat_len == 16
        }) {
            let Some(dev) = Dev::open_feature_only(&c) else {
                continue;
            };
            if ten_read(&dev, 0x13) != Some(1) {
                continue;
            }
            let Some(percent) = ten_read(&dev, 0x12).filter(|&p| p <= 100) else {
                continue;
            };
            let Some(charging) = ten_read(&dev, 0x11) else {
                continue;
            };
            let name = if c.product.is_empty() {
                "Ninjutso Ten / Ten Air".into()
            } else {
                c.product.clone()
            };
            let b = Battery {
                key: ctx.stable_key(&format!("ninjutso-ten-093A:{pid:04X}")),
                name,
                via: if pid == TEN_CABLE {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                percent: Some(percent),
                charging: charging != 0,
                ..Default::default()
            };
            ctx.add(b, vid, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_reply_and_sora_full() {
        let mut r = [0u8; 16];
        r[..2].copy_from_slice(&[0x06, 0x12]);
        r[8] = 58;
        assert_eq!(ten_value(&r, 0x12), Some(58));
        assert_eq!(ten_value(&r, 0x11), None);
        assert_eq!(sora_percent(0), 100);
        assert_eq!(sora_percent(42), 42);
    }
}
