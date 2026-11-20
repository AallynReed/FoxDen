use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1532;
const PID: u16 = 0x053A;
const LEVEL: u8 = 0x21;
const CHARGING: u8 = 0x2A;

fn report(head: &[u8]) -> [u8; 64] {
    let mut r = [0u8; 64];
    r[..head.len()].copy_from_slice(head);
    r
}

fn query(cmd: u8) -> [u8; 64] {
    report(&[0x01, 0x80, 0x08, b'P', b'A', 0x08, 0x08, 0x03, cmd])
}

/// Routes queries from the receiver to the headset; switched off again after reading, nothing is stored.
fn remote(on: bool) -> [u8; 64] {
    report(&[
        0x01, 0x80, 0x07, b'P', b'A', 0x0E, 0x08, 0x02, 0xE1, on as u8,
    ])
}

/// First data byte of the reply to `cmd`.
fn value(cmd: u8, r: &[u8]) -> Option<u8> {
    (r.len() >= 17 && r[13] == cmd && matches!(r[14], 1 | 2) && (1..=32).contains(&r[15]))
        .then(|| r[16])
}

fn drain(dev: &Dev) {
    for _ in 0..8 {
        if dev.read(Duration::from_millis(20)).is_none() {
            break;
        }
    }
}

fn ask(dev: &Dev, cmd: u8) -> Option<u8> {
    for _ in 0..4 {
        drain(dev);
        if !dev.write(&query(cmd)) {
            return None;
        }
        for _ in 0..6 {
            if let Some(v) = dev
                .read(Duration::from_millis(150))
                .and_then(|r| value(cmd, &r))
            {
                return Some(v);
            }
        }
    }
    None
}

pub fn poll_barracuda_pro(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page == 0xFF00 && c.out_len >= 64);
    let Some(dev) = colls.first().and_then(Dev::open) else {
        return;
    };
    let mut on = false;
    for attempt in 1..=3 {
        drain(&dev);
        on = dev.write(&remote(true));
        if on {
            break;
        }
        sleep(Duration::from_millis(150 * attempt));
    }
    let (mut level, mut charging) = (None, None);
    if on {
        sleep(Duration::from_millis(50));
        level = ask(&dev, LEVEL).filter(|&v| v <= 100);
        if level.is_some() {
            charging = ask(&dev, CHARGING);
        }
        drain(&dev);
        dev.write(&remote(false));
    }
    let b = Battery {
        key: ctx.stable_key("razer-1532:053A"),
        name: "Razer Barracuda Pro".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Headset,
        percent: level,
        charging: charging.is_some_and(|c| c > 0),
        connected: level.is_some(),
        ..Battery::default()
    };
    ctx.add(b, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        assert_eq!(
            query(LEVEL)[..9],
            [0x01, 0x80, 0x08, 0x50, 0x41, 0x08, 0x08, 0x03, 0x21]
        );
        assert_eq!(
            remote(false)[..10],
            [0x01, 0x80, 0x07, 0x50, 0x41, 0x0E, 0x08, 0x02, 0xE1, 0x00]
        );
    }

    #[test]
    fn replies() {
        let mut r = [0u8; 64];
        r[..3].copy_from_slice(&[0x01, 0x80, 0x0A]);
        r[3..5].copy_from_slice(b"PI");
        r[13..17].copy_from_slice(&[0x21, 0x02, 0x01, 0x4B]);
        assert_eq!(value(LEVEL, &r), Some(75));
        assert_eq!(value(CHARGING, &r), None);
        r[15] = 0;
        assert_eq!(value(LEVEL, &r), None);
    }
}
