use super::oem_frames::{Power83, query_power83};
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x36A7;

/// `(pid, on a receiver, name, receiver to claim when the cable answers)`; cables first.
const MODELS: [(u16, bool, &str, Option<u16>); 14] = [
    (0xA879, false, "WLMouse Sword X 8K", Some(0xA878)),
    (0xA884, false, "WLMouse Beast X 8K", Some(0xA883)),
    (0xA861, false, "WLMouse Beast G", Some(0xA860)),
    (0xA866, false, "WLMouse mouse", None),
    (0xA867, false, "WLMouse mouse", None),
    (0xA871, false, "WLMouse Beast X Pro", Some(0xA870)),
    (0xA856, true, "WLMouse Beast X V2", None),
    (0xA860, true, "WLMouse Beast G", None),
    (0xA868, true, "WLMouse Beast X Mini Pro", None),
    (0xA870, true, "WLMouse Beast X Pro", None),
    (0xA878, true, "WLMouse Sword X 8K", None),
    (0xA880, true, "WLMouse Beast MAX", None),
    (0xA883, true, "WLMouse Beast X 8K", None),
    (0xA885, true, "WLMouse Beast X Mini", None),
];

pub fn poll_wlmouse(ctx: &mut Ctx) {
    for (pid, on_receiver, name, pair) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let mut colls = ctx.find(VID, pid, |c| c.page == 0xFFFF && c.feat_len >= 65);
        colls.sort_by_key(|c| c.usage != 0);
        let answer = colls
            .iter()
            .filter_map(Dev::open_feature_only)
            .find_map(|d| query_power83(&d, 2, &[60; 6]));
        let Some(answer) = answer else { continue };
        let mut b = Battery {
            key: ctx.stable_key(&format!("wlmouse-36A7:{pid:04X}")),
            name: name.into(),
            via: if on_receiver {
                "2.4 GHz receiver"
            } else {
                "USB cable"
            }
            .into(),
            kind: Kind::Mouse,
            grace_secs: if on_receiver { 1800 } else { 0 },
            ..Default::default()
        };
        match answer {
            Power83::Asleep => b.asleep = true,
            Power83::Level { percent, charging } => {
                b.percent = Some(percent);
                b.charging = charging;
            }
        }
        ctx.add(b, VID, pid);
        if let Some(r) = pair {
            ctx.claim(VID, r);
        }
    }
}
