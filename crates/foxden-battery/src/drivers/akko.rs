use super::oem_frames::{is_status1a_coll, query_status1a};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x38EE;
const MODELS: [(u16, &str); 2] = [(0x000C, "Akko V9 Dash Pro"), (0x000F, "Akko Dash V9 Ultra")];

pub fn poll_akko(ctx: &mut Ctx) {
    for (pid, name) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for c in ctx.find(VID, pid, is_status1a_coll) {
            let Some((percent, charging)) = query_status1a(&c, |l| l <= 100) else {
                continue;
            };
            let b = Battery {
                key: ctx.stable_key(&format!("akko-38EE:{pid:04X}")),
                name: name.into(),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Mouse,
                percent: Some(percent),
                charging,
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, VID, pid);
            break;
        }
    }
}
