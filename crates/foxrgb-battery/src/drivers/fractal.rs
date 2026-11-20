use std::time::Duration;

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x36BC;
const DONGLE: u16 = 0x0001;
const CABLE: u16 = 0x2001;
const REPORT: u8 = 0x02;
const TO_DONGLE: u8 = 0x11;
const TO_HEADSET: u8 = 0xF1;
const DEVICE_STATE: u8 = 0x21;

/// A device-state reply from `target`; `[0]` is the report id.
fn state_reply(r: &[u8], target: u8) -> Option<Vec<u8>> {
    (r.len() >= 24 && r[..3] == [REPORT, target, DEVICE_STATE]).then(|| r.to_vec())
}

fn dongle_linked(r: &[u8]) -> bool {
    r[4] == 1
}

/// (percent, on cable power) from the headset's reply.
fn headset_battery(r: &[u8]) -> Option<(u8, bool)> {
    (r[15] <= 100).then_some((r[15], r[23] == 1))
}

fn ask(dev: &Dev, target: u8) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(&[REPORT, target, DEVICE_STATE]) {
        return None;
    }
    dev.read_until(Duration::from_secs(1), |r| state_reply(r, target))
}

pub fn poll_fractal(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        if pid != DONGLE && pid != CABLE {
            continue;
        }
        let cable = pid == CABLE;
        let colls = ctx.find(VID, pid, |c| {
            c.page == 0xFF00 && c.usage == 0x0001 && c.out_len >= 4 && c.in_len >= 24
        });
        for c in colls {
            let Some(dev) = Dev::open(&c) else { continue };
            let mut b = Battery {
                key: ctx.stable_key("fractal-scape"),
                name: "Fractal Scape".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Headset,
                ..Default::default()
            };
            if !cable {
                let Some(r) = ask(&dev, TO_DONGLE) else {
                    continue;
                };
                // Off, out of range, or on its stand (it unlinks while charging there).
                if !dongle_linked(&r) {
                    b.connected = false;
                    ctx.add(b, VID, pid);
                    break;
                }
            }
            let Some((pct, charging)) = ask(&dev, TO_HEADSET).as_deref().and_then(headset_battery)
            else {
                continue;
            };
            b.percent = Some(pct);
            b.charging = charging;
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(target: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..3].copy_from_slice(&[REPORT, target, DEVICE_STATE]);
        r
    }

    #[test]
    fn dongle_link_flag() {
        let mut r = frame(TO_DONGLE);
        assert!(state_reply(&r, TO_DONGLE).is_some());
        assert!(state_reply(&r, TO_HEADSET).is_none());
        assert!(!dongle_linked(&r));
        r[4] = 1;
        assert!(dongle_linked(&r));
    }

    #[test]
    fn headset_level_and_cable() {
        let mut r = frame(TO_HEADSET);
        r[15] = 62;
        r[23] = 1;
        assert_eq!(headset_battery(&r), Some((62, true)));
        r[15] = 200;
        assert_eq!(headset_battery(&r), None);
    }
}
