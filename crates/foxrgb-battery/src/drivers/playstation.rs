use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::util::{clamp100, hex4};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x054C;
const ADAPTER: u16 = 0x0BA0;

fn is_ds4(pid: u16) -> bool {
    matches!(pid, 0x05C4 | 0x09CC | ADAPTER)
}

fn is_dualsense(pid: u16) -> bool {
    matches!(pid, 0x0CE6 | 0x0DF2)
}

#[derive(Debug, PartialEq)]
enum Parsed {
    Level {
        percent: u8,
        charging: bool,
        full: bool,
    },
    NoPad,
}

/// Battery byte of a regular input report: USB report 01, Bluetooth 11 (DS4) or 31 (DualSense).
fn parse(pid: u16, bt: bool, r: &[u8]) -> Option<Parsed> {
    let ds4 = is_ds4(pid);
    let at = match (ds4, r.first()?) {
        (true, 0x01) => 30,
        (true, 0x11) => 32,
        (false, 0x01) => 53,
        (false, 0x31) => 54,
        _ => return None,
    };
    if r.len() <= at || (r[0] == 0x01 && bt) {
        return None;
    }
    let v = r[at];
    let level = (v & 0x0F) as i32;
    if ds4 {
        if pid == ADAPTER && v == 0 {
            return Some(Parsed::NoPad);
        }
        let cable = v & 0x10 != 0;
        let percent = match (cable, level) {
            (true, 11..) => 100,
            (true, _) => clamp100(level * 10),
            (false, _) => clamp100(level * 10 + 5),
        };
        Some(Parsed::Level {
            percent,
            charging: cable && level < 11,
            full: cable && level >= 11,
        })
    } else {
        let state = v >> 4;
        let full = state == 0x2;
        let percent = if full { 100 } else { clamp100(level * 10 + 5) };
        Some(Parsed::Level {
            percent,
            charging: state == 0x1 || full,
            full,
        })
    }
}

pub fn poll_play_station(ctx: &mut Ctx) {
    let pads: Vec<_> = ctx
        .colls
        .iter()
        .filter(|c| {
            c.vid == VID
                && (is_ds4(c.pid) || is_dualsense(c.pid))
                && c.page == 0x01
                && c.usage == 0x05
        })
        .cloned()
        .collect();
    for c in pads {
        let Some(d) = Dev::open(&c) else { continue };
        let bt = c.in_len > 64;
        if bt {
            // Feature 05 turns on the full Bluetooth reports.
            let _ = d.get_feature(0x05);
        }
        let end = Instant::now() + Duration::from_millis(500);
        let mut got = None;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let Some(r) = d.read(left.min(Duration::from_millis(100))) else {
                continue;
            };
            if r.len() < 10 {
                continue;
            }
            if let Some(p) = parse(c.pid, bt, &r) {
                got = Some(p);
                break;
            }
        }
        let Some(Parsed::Level {
            percent,
            charging,
            full,
        }) = got
        else {
            continue;
        };
        let name = match c.pid {
            p if is_ds4(p) => "DualShock 4",
            0x0DF2 => "DualSense Edge",
            _ => "DualSense",
        };
        let via = match (bt, c.pid) {
            (true, _) => "Bluetooth",
            (false, ADAPTER) => "Wireless adapter",
            _ => "USB cable",
        };
        let b = Battery {
            key: ctx.stable_key(&format!("playstation-{}:{}", hex4(VID), hex4(c.pid))),
            name: name.into(),
            via: via.into(),
            kind: Kind::Gamepad,
            percent: Some(percent),
            charging,
            full,
            ..Battery::default()
        };
        ctx.add(b, VID, c.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u8, at: usize, v: u8, len: usize) -> Vec<u8> {
        let mut r = vec![0u8; len];
        r[0] = id;
        r[at] = v;
        r
    }

    fn lvl(percent: u8, charging: bool, full: bool) -> Option<Parsed> {
        Some(Parsed::Level {
            percent,
            charging,
            full,
        })
    }

    #[test]
    fn dualshock_levels() {
        assert_eq!(
            parse(0x09CC, false, &report(0x01, 30, 0x07, 64)),
            lvl(75, false, false)
        );
        assert_eq!(
            parse(0x09CC, false, &report(0x01, 30, 0x15, 64)),
            lvl(50, true, false)
        );
        assert_eq!(
            parse(0x09CC, false, &report(0x01, 30, 0x1B, 64)),
            lvl(100, false, true)
        );
        assert_eq!(
            parse(0x09CC, true, &report(0x11, 32, 0x03, 78)),
            lvl(35, false, false)
        );
        assert_eq!(parse(0x09CC, true, &report(0x01, 30, 0x03, 64)), None);
        assert_eq!(
            parse(ADAPTER, false, &report(0x01, 30, 0, 64)),
            Some(Parsed::NoPad)
        );
    }

    #[test]
    fn dualsense_levels() {
        assert_eq!(
            parse(0x0CE6, false, &report(0x01, 53, 0x04, 64)),
            lvl(45, false, false)
        );
        assert_eq!(
            parse(0x0CE6, true, &report(0x31, 54, 0x18, 78)),
            lvl(85, true, false)
        );
        assert_eq!(
            parse(0x0DF2, false, &report(0x01, 53, 0x2A, 64)),
            lvl(100, true, true)
        );
    }
}
