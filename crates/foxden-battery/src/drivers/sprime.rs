use super::oem_frames::{Status15, query_status15};
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1915;
const CABLES: [u16; 3] = [0xAC11, 0xAC12, 0xAC13];
const RECEIVERS: [u16; 2] = [0xAC1C, 0xAD8C];

/// `(percent, charging)` once the mouse is reachable.
fn reading(s: &Status15, cable: bool) -> Option<(u8, bool)> {
    let r = if s.full {
        (100, false)
    } else {
        (s.percent, s.charging)
    };
    (cable || s.online).then_some(r)
}

pub fn poll_sprime(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        let cable = CABLES.contains(&pid);
        if !cable && !RECEIVERS.contains(&pid) {
            continue;
        }
        for c in ctx.find(vid, pid, |c| {
            c.page == 0xFFA0 && c.usage == 1 && c.feat_len >= 32
        }) {
            let Some(dev) = Dev::open_feature_only(&c) else {
                continue;
            };
            let Some(s) = query_status15(&dev, cable) else {
                continue;
            };
            let r = reading(&s, cable);
            let b = Battery {
                key: ctx.stable_key(&format!("sprime-1915:{pid:04X}")),
                name: "Sprime PM1".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                connected: r.is_some(),
                percent: r.map(|r| r.0),
                charging: r.is_some_and(|r| r.1),
                full: r.is_some() && s.full,
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
    fn full_wins_over_charging() {
        let s = Status15 {
            percent: 97,
            charging: true,
            full: true,
            online: true,
        };
        assert_eq!(reading(&s, false), Some((100, false)));
        let off = Status15 { online: false, ..s };
        assert_eq!(reading(&off, false), None);
        assert_eq!(reading(&off, true), Some((100, false)));
    }
}
