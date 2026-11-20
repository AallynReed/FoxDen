use std::time::Duration;

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x047F;
const PIDS: [u16; 2] = [0x02EE, 0x02F7];
const REPORT: u8 = 0x07;
const TO_ADAPTER: u8 = 0x00;
const TO_HEADSET: u8 = 0x20;
const FROM_ADAPTER: u8 = 0x00;
const FROM_HEADSET: u8 = 0x02;
const HELLO: u8 = 0x01;
const GET: u8 = 0x02;
const GET_REPLY: u8 = 0x03;
const HELLO_ACK: u8 = 0x08;
/// (group, item) of the hello and of the battery status.
const HELLO_ITEM: (u8, u8) = (0x01, 0x02);
const BATTERY_ITEM: (u8, u8) = (0x0A, 0x1A);

fn packet(op: u8, (group, item): (u8, u8), addr: u8) -> [u8; 11] {
    let len = if op == GET { 0x06 } else { 0x07 };
    [
        REPORT, 0x01, 0x01, 0x10, len, addr, 0x00, 0x00, op, group, item,
    ]
}

fn answers(r: &[u8], op: u8, (group, item): (u8, u8), addr: u8) -> bool {
    r.len() > 10
        && r[..4] == [REPORT, 0x01, 0x01, 0x10]
        && r[5] == addr
        && r[8] == op
        && r[9] == group
        && r[10] == item
}

/// Battery reply `[11]` level, `[12]` level count, `[13]` charging.
fn parse_battery(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 14 {
        return None;
    }
    let (level, count) = (r[11] as u32, r[12] as u32);
    (count >= 2 && level < count).then(|| ((level * 100 / (count - 1)) as u8, r[13] != 0))
}

fn wait_for(d: &Dev, op: u8, item: (u8, u8), addr: u8) -> Option<Vec<u8>> {
    for _ in 0..16 {
        let r = d
            .read(Duration::from_millis(1000))
            .filter(|r| !r.is_empty())?;
        if answers(&r, op, item, addr) {
            return Some(r);
        }
    }
    None
}

fn fetch(d: &Dev) -> Option<(u8, bool)> {
    d.flush();
    // Attach handshake before the GET.
    d.write(&[0x06, 0x01]);
    d.write(&packet(HELLO, HELLO_ITEM, TO_ADAPTER));
    wait_for(d, HELLO_ACK, HELLO_ITEM, FROM_ADAPTER);
    if !d.write(&packet(HELLO, HELLO_ITEM, TO_HEADSET)) {
        return None;
    }
    wait_for(d, HELLO_ACK, HELLO_ITEM, FROM_HEADSET)?;
    if !d.write(&packet(GET, BATTERY_ITEM, TO_HEADSET)) {
        return None;
    }
    parse_battery(&wait_for(d, GET_REPLY, BATTERY_ITEM, FROM_HEADSET)?)
}

pub fn poll_plantronics(ctx: &mut Ctx) {
    for pid in PIDS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for c in ctx.find(VID, pid, |c| c.page == 0xFFA2 && c.usage == 0x0003) {
            let Some((percent, charging)) = Dev::open(&c).and_then(|d| fetch(&d)) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&format!("plantronics-{}:{}", hex4(VID), hex4(pid))),
                name: "Poly / Plantronics headset".into(),
                via: "BT600 adapter".into(),
                kind: Kind::Headset,
                percent: Some(percent.min(100)),
                charging,
                ..Battery::default()
            };
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_scale_and_matching() {
        assert_eq!(
            packet(GET, BATTERY_ITEM, TO_HEADSET),
            [0x07, 1, 1, 0x10, 6, 0x20, 0, 0, 2, 0x0A, 0x1A]
        );
        let mut r = [0u8; 64];
        r[..11].copy_from_slice(&[
            0x07,
            1,
            1,
            0x10,
            9,
            FROM_HEADSET,
            0,
            0,
            GET_REPLY,
            0x0A,
            0x1A,
        ]);
        r[11..14].copy_from_slice(&[3, 5, 1]);
        assert!(answers(&r, GET_REPLY, BATTERY_ITEM, FROM_HEADSET));
        assert!(!answers(&r, HELLO_ACK, HELLO_ITEM, FROM_HEADSET));
        assert_eq!(parse_battery(&r), Some((75, true)));
        r[11] = 5;
        assert_eq!(parse_battery(&r), None);
    }
}
