use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3367;
const DONGLE_4K: u16 = 0x1970;

const OK: u8 = 0x01;
const BUSY: u8 = 0x03;

/// The mouse names itself only while awake, so the last name is kept for sleeping polls.
static LAST_NAME: Mutex<Option<&'static str>> = Mutex::new(None);

fn mouse_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x1972 => "Endgame Gear OP1w 4k",
        0x1984 => "Endgame Gear OP1w 4k v2",
        0x1968 => "Endgame Gear XM2w 4k",
        0x1982 => "Endgame Gear XM2w 4k v2",
        _ => return None,
    })
}

fn asleep_status(st: u8) -> bool {
    matches!(st, 0x08 | 0x11)
}

/// Mouse PID from a 0E (mouse info) reply that carries the Endgame vendor id.
fn info_pid(r: &[u8]) -> Option<u16> {
    (r.len() >= 20 && r[16..18] == [0x67, 0x33]).then(|| u16::from_le_bytes([r[18], r[19]]))
}

/// Feature report A1 `[A1][cmd][target][00...]`; returns the status byte and the reply.
fn command(dev: &Dev, cmd: u8, target: u8) -> Option<(u8, Vec<u8>)> {
    let mut req = [0u8; 64];
    req[..3].copy_from_slice(&[0xA1, cmd, target]);
    if !dev.set_feature(&req) {
        return None;
    }
    let mut last = None;
    for attempt in 0..3 {
        sleep(Duration::from_millis(150 + attempt * 100));
        let r = dev
            .get_feature(0xA1)
            .filter(|r| r.len() >= 20 && r[0] == 0xA1)?;
        let st = r[1];
        last = Some((st, r));
        if st != BUSY {
            break;
        }
    }
    last
}

pub fn poll_endgame(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, DONGLE_4K) {
        return;
    }
    let colls = ctx.find(VID, DONGLE_4K, |c| {
        c.page == 0xFF01 && c.usage == 0x0002 && c.feat_len >= 64
    });
    for c in colls {
        let Some(dev) = Dev::open_feature_only(&c) else {
            continue;
        };
        let mut reply = command(&dev, 0xB4, 0x00);
        if !reply
            .as_ref()
            .is_some_and(|(st, _)| *st == OK || asleep_status(*st))
        {
            // After a failed command the dongle stays silent until a 0F probe of dongle and mouse.
            command(&dev, 0x0F, 0x01);
            command(&dev, 0x0F, 0x0F);
            reply = command(&dev, 0xB4, 0x00);
        }
        let Some((st, r)) = reply else { continue };
        let percent = (st == OK && r[16] <= 100).then_some(r[16]);
        let asleep = asleep_status(st);
        if percent.is_none() && !asleep {
            continue;
        }
        let mut last = LAST_NAME.lock().unwrap_or_else(|e| e.into_inner());
        if percent.is_some()
            && let Some((OK, info)) = command(&dev, 0x0E, 0x00)
            && let Some(name) = info_pid(&info).and_then(mouse_name)
        {
            *last = Some(name);
        }
        let b = Battery {
            key: ctx.stable_key("endgame-3367:1970"),
            name: last.unwrap_or("Endgame Gear 4k wireless mouse").into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent,
            asleep,
            ..Default::default()
        };
        drop(last);
        ctx.add(b, VID, DONGLE_4K);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_info_names_the_mouse() {
        let mut r = vec![0u8; 64];
        r[0] = 0xA1;
        r[1] = OK;
        r[16..20].copy_from_slice(&[0x67, 0x33, 0x84, 0x19]);
        assert_eq!(
            info_pid(&r).and_then(mouse_name),
            Some("Endgame Gear OP1w 4k v2")
        );
        r[16] = 0;
        assert_eq!(info_pid(&r), None);
    }
}
