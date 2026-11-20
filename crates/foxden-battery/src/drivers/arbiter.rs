use super::oem_frames::{Power83, query_power83};
use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x373E;

struct Akitsu {
    cable: u16,
    receiver: u16,
    name: &'static str,
}

const MODELS: [Akitsu; 3] = [
    Akitsu {
        cable: 0x0009,
        receiver: 0x000A,
        name: "Arbiter Studio Akitsu",
    },
    Akitsu {
        cable: 0x0038,
        receiver: 0x0039,
        name: "Arbiter Studio Akitsu 3950",
    },
    Akitsu {
        cable: 0x0066,
        receiver: 0x0067,
        name: "Arbiter Studio Akitsu Medium",
    },
];

fn is_vendor(c: &Coll) -> bool {
    c.page == 0xFFFF && c.feat_len == 65
}

/// Runs before the LAMZU driver, which would otherwise list these 373E mice under its own name.
pub fn poll_arbiter(ctx: &mut Ctx) {
    // Cables first: an answering cable also claims its receiver.
    let cables = MODELS.iter().map(|m| (m, m.cable, false));
    let receivers = MODELS.iter().map(|m| (m, m.receiver, true));
    for (m, pid, on_receiver) in cables.chain(receivers) {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let mut colls = ctx.find(VID, pid, is_vendor);
        if colls.is_empty() {
            continue;
        }
        colls.sort_by_key(|c| c.usage != 0);
        let answer = colls
            .iter()
            .filter_map(Dev::open_feature_only)
            .find_map(|d| query_power83(&d, 2, &[100, 60, 60]));
        let Some(answer) = answer else {
            // Silent hardware is still ours, not LAMZU's.
            ctx.claim(VID, pid);
            continue;
        };
        let pair_present = on_receiver || !ctx.find(VID, m.receiver, is_vendor).is_empty();
        let mut b = Battery {
            key: ctx.stable_key(&format!("arbiter-373E:{:04X}", m.receiver)),
            name: m.name.into(),
            via: if on_receiver {
                "2.4 GHz receiver"
            } else {
                "USB cable"
            }
            .into(),
            kind: Kind::Mouse,
            grace_secs: if pair_present { 1800 } else { 0 },
            ..Default::default()
        };
        match answer {
            Power83::Asleep => b.asleep = true,
            Power83::Level { percent, charging } => {
                // 0 through the receiver: mouse off or out of range
                b.connected = percent > 0 || !on_receiver;
                b.percent = b.connected.then_some(percent);
                b.charging = charging;
            }
        }
        ctx.add(b, VID, pid);
        ctx.claim(VID, m.receiver);
    }
}
