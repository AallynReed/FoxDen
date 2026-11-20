use std::time::Duration;

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x2516;
const DONGLE: u16 = 0x01B7;
const MM712_CABLE: u16 = 0x0169;
const MM731_CABLE: u16 = 0x0167;

/// Status report `47 81 [model] ... [10] percent [11] charging [12] link (0 cable)`.
#[derive(Debug, PartialEq)]
struct Status {
    model: u8,
    percent: Option<u8>,
    charging: bool,
    cable: bool,
}

fn parse_status(r: &[u8]) -> Option<Status> {
    if r.len() < 13 || r[0] != 0x47 || r[1] != 0x81 {
        return None;
    }
    Some(Status {
        model: r[2],
        percent: (1..=100).contains(&r[10]).then_some(r[10]),
        charging: r[11] != 0,
        cable: r[12] == 0,
    })
}

fn model_name(pid: u16, model: u8) -> &'static str {
    match (pid, model) {
        (MM712_CABLE, _) | (DONGLE, 0x50) => "Cooler Master MM712",
        (MM731_CABLE, _) | (DONGLE, 0x70) => "Cooler Master MM731",
        _ => "Cooler Master wireless mouse",
    }
}

pub fn poll_cooler_master(ctx: &mut Ctx) {
    for (_, pid) in ctx.products(VID) {
        if !matches!(pid, DONGLE | MM712_CABLE | MM731_CABLE) {
            continue;
        }
        for c in ctx.find(VID, pid, |c| c.iface == 3 && c.page == 0xFF0A) {
            let Some(d) = Dev::open(&c) else { continue };
            d.flush();
            if !d.write(&[0x00, 0x46, 0x81, 0x00]) {
                continue;
            }
            let Some(s) = d.read_until(Duration::from_millis(500), parse_status) else {
                continue;
            };
            let name = model_name(pid, s.model);
            let mut dup = false;
            for o in ctx.out.iter_mut().filter(|o| o.name == name) {
                dup = true;
                o.charging |= s.charging;
            }
            if dup {
                ctx.claim(VID, pid);
            } else {
                let b = Battery {
                    key: ctx.stable_key(&format!("coolermaster-{}:{}", hex4(VID), hex4(pid))),
                    name: name.into(),
                    via: if s.cable {
                        "USB cable"
                    } else {
                        "2.4 GHz receiver"
                    }
                    .into(),
                    kind: Kind::Mouse,
                    percent: s.percent,
                    charging: s.charging,
                    grace_secs: 1800,
                    ..Battery::default()
                };
                ctx.add(b, VID, pid);
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_report_decodes() {
        let mut r = [0u8; 32];
        r[..3].copy_from_slice(&[0x47, 0x81, 0x50]);
        r[10] = 64;
        r[12] = 1;
        let s = parse_status(&r).unwrap();
        assert_eq!(
            s,
            Status {
                model: 0x50,
                percent: Some(64),
                charging: false,
                cable: false
            }
        );
        assert_eq!(model_name(DONGLE, s.model), "Cooler Master MM712");
        r[10] = 0;
        assert_eq!(parse_status(&r).unwrap().percent, None);
        assert!(parse_status(&[0x44, 0x81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
    }
}
