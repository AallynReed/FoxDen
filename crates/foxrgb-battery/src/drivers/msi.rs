use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::util::clamp100;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x0DB0;

#[derive(Clone, Copy, PartialEq)]
enum Link {
    /// Answers a `0F 20` status request.
    Asked,
    /// Only pushes frames while the mouse moves.
    Pushed,
}

struct Model {
    pid: u16,
    cable: bool,
    link: Link,
    page: u16,
    usage: u16,
    name: &'static str,
}

const GM31: &str = "MSI Clutch GM31 Lightweight Wireless";
const GM41: &str = "MSI Clutch GM41 Lightweight Wireless";

const MODELS: &[Model] = &[
    Model {
        pid: 0x9E22,
        cable: false,
        link: Link::Asked,
        page: 0xFF0D,
        usage: 0x00B0,
        name: "MSI VERSA 300 ELITE WIRELESS",
    },
    Model {
        pid: 0x0D3C,
        cable: false,
        link: Link::Asked,
        page: 0xFF00,
        usage: 0x0001,
        name: GM31,
    },
    Model {
        pid: 0x0D3B,
        cable: true,
        link: Link::Asked,
        page: 0xFF00,
        usage: 0x0001,
        name: GM31,
    },
    Model {
        pid: 0x0D4C,
        cable: false,
        link: Link::Pushed,
        page: 0xFF10,
        usage: 0x0006,
        name: GM41,
    },
    Model {
        pid: 0x0D4B,
        cable: true,
        link: Link::Pushed,
        page: 0xFF10,
        usage: 0x0006,
        name: GM41,
    },
];

fn fits(m: &Model, c: &Coll) -> bool {
    c.iface == 1
        && c.page == m.page
        && c.usage == m.usage
        && match m.link {
            Link::Asked => c.in_len >= 65 && c.out_len >= 65,
            Link::Pushed => c.in_len >= 7,
        }
}

/// Status frame `0F 20 ..`; byte 14 is charging (bit 7) and percent.
fn parse_status(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 15 || r[0] != 0x0F || r[1] != 0x20 {
        return None;
    }
    let pct = r[14] & 0x7F;
    (pct <= 100).then_some((pct, r[14] & 0x80 != 0))
}

/// Pushed frame `A0 raw 07 flags ..`: (raw level, flags).
fn parse_push(r: &[u8]) -> Option<(u8, u8)> {
    (r.len() >= 7 && r[0] == 0xA0 && r[2] == 0x07).then(|| (r[1], r[3]))
}

/// Linear fit of the raw level, about half a percent per step.
fn push_percent(raw: u8) -> u8 {
    clamp100((raw as i32 * 51 + 1810) / 100)
}

fn ask(c: &Coll) -> Option<(u8, bool)> {
    let dev = Dev::open(c)?;
    dev.flush();
    if !dev.write(&[0x00, 0x0F, 0x20]) {
        return None;
    }
    dev.read_until(Duration::from_millis(1000), parse_status)
}

fn listen(c: &Coll) -> Option<(u8, u8)> {
    Dev::open_read_only(c)?.read_until(Duration::from_millis(2000), parse_push)
}

pub fn poll_msi(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, 0x9E22) {
        return;
    }
    for m in MODELS {
        if ctx.is_claimed(VID, m.pid) {
            continue;
        }
        let Some(c) = ctx.find(VID, m.pid, |c| fits(m, c)).into_iter().next() else {
            continue;
        };
        let (pct, charging, detail) = match m.link {
            Link::Asked => {
                let Some((p, ch)) = ask(&c) else { continue };
                (p, ch, String::new())
            }
            Link::Pushed => {
                let Some((raw, flags)) = listen(&c) else {
                    continue;
                };
                (
                    push_percent(raw),
                    false,
                    format!("approximate (raw 0x{raw:02X}, flags 0x{flags:02X})"),
                )
            }
        };
        // 0% on battery means the mouse is off or asleep.
        let asleep = pct == 0 && !charging;
        let b = Battery {
            key: ctx.stable_key(&format!("msi-{VID:04X}:{:04X}", m.pid)),
            name: m.name.into(),
            via: if m.cable {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Mouse,
            percent: (!asleep).then_some(pct),
            charging: !asleep && charging,
            asleep,
            detail,
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, m.pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_frame_from_a_gm31() {
        let r = [
            0x0F, 0x20, 0, 0, 0x23, 0x05, 0x7A, 0x91, 0, 0, 0, 0, 0x05, 0x80, 0x56, 0x00,
        ];
        assert_eq!(parse_status(&r), Some((86, false)));
        let mut c = r;
        c[14] = 0xD0;
        assert_eq!(parse_status(&c), Some((80, true)));
        c[1] = 0x21;
        assert_eq!(parse_status(&c), None);
    }

    #[test]
    fn pushed_level_maps_to_percent() {
        assert_eq!(
            parse_push(&[0xA0, 0x4D, 0x07, 0x01, 0x94, 0x00, 0x94]),
            Some((0x4D, 0x01))
        );
        assert_eq!(
            parse_push(&[0xA0, 0x4D, 0x06, 0x01, 0x94, 0x00, 0x94]),
            None
        );
        assert_eq!(push_percent(0x4D), 57);
        assert_eq!(push_percent(0x48), 54);
        assert_eq!(push_percent(0x45), 53);
    }
}
