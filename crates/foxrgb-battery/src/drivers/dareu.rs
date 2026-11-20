use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x260D;

// --- Togran: receiver 1131 (mice listed by slot) and the A980 Pro cable 1148 ---

const TOGRAN_RECEIVER: u16 = 0x1131;
const A980_PRO_CABLE: u16 = 0x1148;
const TOGRAN_REPORT: u8 = 0x0A;

fn togran_request(slot: u8, len: u8, class: u8, cmd: u8) -> Vec<u8> {
    let mut r = vec![0u8; 65];
    r[..5].copy_from_slice(&[TOGRAN_REPORT, slot << 4, len, class, cmd]);
    r
}

/// A finished reply echoes the class and command with status 2 in the low nibble of byte 1.
fn togran_done(r: &[u8], class: u8, cmd: u8) -> bool {
    r.len() >= 11 && r[1] & 0x0F == 2 && r[3] == class && r[4] == cmd
}

fn togran_query(dev: &Dev, slot: u8, len: u8, class: u8, cmd: u8) -> Option<Vec<u8>> {
    if !dev.set_feature(&togran_request(slot, len, class, cmd)) {
        return None;
    }
    for _ in 0..15 {
        sleep(Duration::from_millis(30));
        let r = dev.get_feature(TOGRAN_REPORT)?;
        if r.len() < 11 {
            return None;
        }
        if togran_done(&r, class, cmd) {
            return Some(r);
        }
    }
    None
}

/// Mouse pids of the occupied slots 1..=5 in a pairing list reply.
fn togran_slots(r: &[u8]) -> Vec<(u8, u16)> {
    let count = r.get(7).copied().unwrap_or(0).min(5);
    (1..=count)
        .filter_map(|slot| {
            let e = r.get(8 + (slot as usize - 1) * 3..8 + slot as usize * 3)?;
            (e[0] != 0).then(|| (slot, u16::from_be_bytes([e[1], e[2]])))
        })
        .collect()
}

/// Percent and charging from a battery reply: [7] status (1 charging), [8] level, [9] scale.
fn togran_battery(r: &[u8]) -> Option<(u8, bool)> {
    let max = if r[9] == 0 { 100 } else { r[9] };
    (r[8] <= max).then(|| ((u32::from(r[8]) * 100 / u32::from(max)) as u8, r[7] == 1))
}

fn add_togran(ctx: &mut Ctx, dev: &Dev, slot: u8, pid: u16, mouse: u16) {
    let Some((pct, charging)) = togran_query(dev, slot, 4, 0x07, 0x80)
        .as_deref()
        .and_then(togran_battery)
    else {
        return;
    };
    let base = if pid == mouse {
        format!("dareu-{VID:04X}:{pid:04X}")
    } else {
        format!("dareu-{VID:04X}:{pid:04X}:{mouse:04X}")
    };
    let b = Battery {
        key: ctx.stable_key(&base),
        name: if mouse == A980_PRO_CABLE {
            "DAREU A980 Pro".into()
        } else {
            format!("DAREU {mouse:04X}")
        },
        via: if slot == 0 {
            "USB cable"
        } else {
            "2.4 GHz receiver"
        }
        .into(),
        kind: Kind::Mouse,
        percent: Some(pct),
        charging,
        ..Default::default()
    };
    ctx.add(b, VID, pid);
}

fn poll_togran(ctx: &mut Ctx) {
    for pid in [TOGRAN_RECEIVER, A980_PRO_CABLE] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.page == 0x000C && c.usage == 1 && c.feat_len == 65
        });
        let [coll] = colls.as_slice() else { continue };
        let Some(dev) = Dev::open_feature_only(coll) else {
            continue;
        };
        if pid == A980_PRO_CABLE {
            add_togran(ctx, &dev, 0, pid, pid);
            continue;
        }
        // A wired mouse stays listed with status 0 here, so it is not shown twice.
        let Some(list) = togran_query(&dev, 0, 6, 0x00, 0xA0) else {
            continue;
        };
        for (slot, mouse) in togran_slots(&list) {
            add_togran(ctx, &dev, slot, pid, mouse);
        }
    }
}

// --- A84 keyboard (SONiX, 260D:0022) ---

const A84: u16 = 0x0022;

/// Level from a feature report 0 reply; byte 15 stays 0 until the answer is in.
fn a84_level(r: &[u8]) -> Option<Option<u8>> {
    if r.len() < 16 || r[15] == 0 {
        return None;
    }
    Some((r[5] <= 100).then_some(r[5]))
}

fn poll_a84(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, A84) {
        return;
    }
    let colls = ctx.find(VID, A84, |c| {
        c.iface == 0 && c.page == 0x01 && c.usage == 0x06 && c.feat_len == 65
    });
    let [coll] = colls.as_slice() else { return };
    let Some(dev) = Dev::open_feature_only(coll) else {
        return;
    };
    let mut req = vec![0u8; 65];
    req[1] = 0x04;
    req[2] = 0xF6;
    if !dev.set_feature(&req) {
        return;
    }
    for i in 0..6 {
        sleep(Duration::from_millis(10));
        let r = dev.get_feature(0);
        // The first three reads after the request carry stale data.
        if i < 3 {
            continue;
        }
        let Some(level) = r.as_deref().and_then(a84_level) else {
            continue;
        };
        let Some(pct) = level else { return };
        let b = Battery {
            key: ctx.stable_key(&format!("dareu-{VID:04X}:{A84:04X}")),
            name: "DAREU A84".into(),
            via: "USB".into(),
            kind: Kind::Keyboard,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, VID, A84);
        return;
    }
}

// --- EM901X (receiver 1074, cable 1090) and Sharkoon Skiller SGM50W (1125) ---

const EM901X_CABLE: u16 = 0x1090;
const EM901X_RECEIVER: u16 = 0x1074;
const SGM50W: u16 = 0x1125;

/// Input report 09 answering command 04: [2] status 0, [6] percent, [7] charging.
fn em901x_status(r: &[u8]) -> Option<(u8, bool)> {
    (r.len() == 17 && r[0] == 0x09 && r[1] == 0x04 && r[2] == 0 && r[6] <= 100)
        .then(|| (r[6], r[7] != 0))
}

fn poll_em901x(ctx: &mut Ctx) {
    let mut cable_read = false;
    for pid in [EM901X_CABLE, EM901X_RECEIVER, SGM50W] {
        // A cable reading beats the receiver's cached telemetry.
        if ctx.is_claimed(VID, pid) || (cable_read && pid == EM901X_RECEIVER) {
            continue;
        }
        let inputs = ctx.find(VID, pid, |c| {
            c.iface == 1 && c.page == 0xFF01 && c.usage == 0 && c.in_len == 17
        });
        let features = ctx.find(VID, pid, |c| {
            c.iface == 1 && c.page == 0xFF02 && c.usage == 2 && c.feat_len == 17
        });
        // Several identical devices: no safe way to pair the two collections.
        let ([input], [feature]) = (inputs.as_slice(), features.as_slice()) else {
            continue;
        };
        let (Some(input), Some(feature)) = (Dev::open(input), Dev::open_feature_only(feature))
        else {
            continue;
        };
        input.flush();
        // The request goes out as feature report 08, the reply comes back as input 09 on the sibling.
        if !feature.set_feature(&super::zenith::request()) {
            continue;
        }
        let Some((pct, charging)) = input.read_until(Duration::from_secs(1), em901x_status) else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("dareu-{VID:04X}:{pid:04X}")),
            name: if pid == SGM50W {
                "Sharkoon Skiller SGM50W"
            } else {
                "DAREU EM901X"
            }
            .into(),
            via: if pid == EM901X_CABLE {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            charging,
            ..Default::default()
        };
        ctx.add(b, VID, pid);
        cable_read = pid == EM901X_CABLE;
    }
}

pub fn poll_dareu(ctx: &mut Ctx) {
    poll_togran(ctx);
    poll_a84(ctx);
    poll_em901x(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn togran_pairing_list_and_battery() {
        let mut list = vec![0u8; 65];
        list[..8].copy_from_slice(&[0x0A, 0x02, 6, 0x00, 0xA0, 0, 0, 2]);
        list[8..14].copy_from_slice(&[1, 0x11, 0x48, 0, 0x11, 0x50]);
        assert!(togran_done(&list, 0x00, 0xA0));
        assert_eq!(togran_slots(&list), vec![(1, 0x1148)]);

        let mut bat = vec![0u8; 65];
        bat[..10].copy_from_slice(&[0x0A, 0x12, 4, 0x07, 0x80, 0, 0, 1, 8, 10]);
        assert_eq!(togran_battery(&bat), Some((80, true)));
        bat[9] = 0;
        bat[7] = 0;
        assert_eq!(togran_battery(&bat), Some((8, false)));
    }

    #[test]
    fn a84_waits_for_byte_15() {
        let mut r = vec![0u8; 65];
        r[5] = 64;
        assert_eq!(a84_level(&r), None);
        r[15] = 1;
        assert_eq!(a84_level(&r), Some(Some(64)));
        r[5] = 200;
        assert_eq!(a84_level(&r), Some(None));
    }

    #[test]
    fn em901x_reply() {
        let mut r = vec![0u8; 17];
        r[..8].copy_from_slice(&[0x09, 0x04, 0, 0, 0, 0, 91, 1]);
        assert_eq!(em901x_status(&r), Some((91, true)));
        r[2] = 1;
        assert_eq!(em901x_status(&r), None);
    }
}
