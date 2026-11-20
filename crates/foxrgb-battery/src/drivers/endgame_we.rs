use std::time::Duration;

use super::oem_frames::{frame55, sum55_ok};
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3367;
const XM2WE_RECEIVER: u16 = 0x1961;
const OP_LINK: u8 = 0x03;
const OP_BATTERY: u8 = 0x04;

/// Payload of an input report 09 answering `op`: `[09][op][00 00 00][size][payload]`.
fn payload(r: &[u8], op: u8) -> Option<&[u8]> {
    if r.len() < 17 || r[0] != 0x09 || r[1] != op || r[2..5] != [0, 0, 0] || !sum55_ok(&r[..17]) {
        return None;
    }
    let size = r[5] as usize;
    (1..=10).contains(&size).then(|| &r[6..6 + size])
}

/// The two collections of one receiver differ only in `&colNN` and the last `&` part of the instance.
fn receiver_id(path: &str) -> String {
    let p = path.to_lowercase();
    let parts: Vec<&str> = p.split('#').collect();
    if parts.len() < 4 {
        return p;
    }
    let mut hw = parts[1].to_string();
    if let Some(i) = hw.find("&col") {
        hw.replace_range(i..(i + 6).min(hw.len()), "");
    }
    let inst = parts[2].rsplit_once('&').map_or(parts[2], |(head, _)| head);
    format!("{hw}#{inst}")
}

fn query(feat: &Dev, input: &Dev, op: u8) -> Option<Vec<u8>> {
    input.flush();
    if !feat.set_feature(&frame55(0x08, op)) {
        return None;
    }
    input.read_until(Duration::from_millis(400), |r| {
        payload(r, op).map(<[u8]>::to_vec)
    })
}

pub fn poll_endgame_we(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, XM2WE_RECEIVER) {
        return;
    }
    let inputs = ctx.find(VID, XM2WE_RECEIVER, |c| {
        c.page == 0xFF01 && c.usage == 0 && c.in_len >= 17
    });
    let feats = ctx.find(VID, XM2WE_RECEIVER, |c| {
        c.page == 0xFF02 && c.usage == 0x0002 && c.feat_len >= 17
    });
    for f in feats {
        let id = receiver_id(&f.path);
        let Some(i) = inputs.iter().find(|i| receiver_id(&i.path) == id) else {
            continue;
        };
        let (Some(feat), Some(input)) = (Dev::open_feature_only(&f), Dev::open(i)) else {
            continue;
        };
        // Battery first: the link can drop as the charger goes in.
        let Some(battery) = query(&feat, &input, OP_BATTERY).filter(|b| b[0] <= 100) else {
            continue;
        };
        let linked = query(&feat, &input, OP_LINK).is_some_and(|l| l[0] == 1);
        let charging = battery.get(1) == Some(&1);
        let awake = charging || linked;
        let b = Battery {
            key: ctx.stable_key("endgame-3367:1961"),
            name: "Endgame Gear XM2we".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: awake.then_some(battery[0]),
            charging: awake && charging,
            asleep: !awake,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, XM2WE_RECEIVER);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_payload() {
        let mut r = [0u8; 17];
        r[..8].copy_from_slice(&[0x09, OP_BATTERY, 0, 0, 0, 2, 81, 1]);
        r[16] = 0x55u8.wrapping_sub(r[..16].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
        assert_eq!(payload(&r, OP_BATTERY), Some(&[81u8, 1][..]));
        assert_eq!(payload(&r, OP_LINK), None);
    }

    #[test]
    fn collections_of_one_receiver_match() {
        let a = r"\\?\HID#VID_3367&PID_1961&MI_01&Col01#8&1a2b3c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let b = r"\\?\HID#VID_3367&PID_1961&MI_01&Col02#8&1a2b3c&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(receiver_id(a), receiver_id(b));
    }
}
