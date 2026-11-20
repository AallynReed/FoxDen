use std::thread::sleep;
use std::time::Duration;

use super::oem_frames::{Power83, parse_power83, power83_request};
use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const MODELS: &[(u16, &[u16], &str)] = &[
    (0x373E, &[0x001C, 0x001D, 0x001E], "LAMZU Maya X"),
    (0x373E, &[0x0016], "LAMZU Maya X V2"),
    (0x37B0, &[0x001F], "LAMZU Maya X V2"),
    (0x37B0, &[0x0005, 0x000B, 0x000C], "LAMZU Tachi"),
    (0x37B0, &[0x001C, 0x001E], "LAMZU Tachi Lite"),
    (0x37B0, &[0x0009, 0x000F, 0x0010], "LAMZU Inca"),
    (0x37B0, &[0x0011, 0x0013, 0x0015], "LAMZU Maya"),
    (0x37B0, &[0x0007, 0x000D, 0x000E], "LAMZU Paro"),
    (0x37B0, &[0x0017, 0x0019, 0x001B], "LAMZU Thorn"),
    (
        0x37B0,
        &[0x0021, 0x0003, 0x0023, 0x0030, 0x0032, 0x0040],
        "LAMZU Thorn V2",
    ),
    (0x37B0, &[0x0025, 0x0027], "LAMZU Atlantis OG Champion"),
    (
        0x37B0,
        &[0x0028, 0x002B, 0x002A, 0x003E, 0x003F],
        "LAMZU Atlantis Mini",
    ),
    (0x37B0, &[0x002C, 0x0052], "LAMZU Maya M"),
    (0x37B0, &[0x003A, 0x003B, 0x0044], "LAMZU Maya X"),
    (0x37B0, &[0x003C, 0x0050], "LAMZU Maya"),
    (0x37B0, &[0x0042, 0x0056], "LAMZU Orcus"),
    (0x37B0, &[0x0046, 0x0054], "LAMZU Mini"),
];

const CABLES_37B0: [u16; 24] = [
    0x0005, 0x0007, 0x0009, 0x0011, 0x0017, 0x001C, 0x001F, 0x0021, 0x0025, 0x0028, 0x002C, 0x0030,
    0x0034, 0x0038, 0x003A, 0x003C, 0x003E, 0x0040, 0x0042, 0x0044, 0x0046, 0x0050, 0x0052, 0x0056,
];

fn model_name(vid: u16, pid: u16) -> &'static str {
    MODELS
        .iter()
        .find(|(v, pids, _)| *v == vid && pids.contains(&pid))
        .map_or("LAMZU wireless mouse", |m| m.2)
}

fn is_cable(vid: u16, pid: u16) -> bool {
    if vid == 0x373E {
        pid == 0x001C
    } else {
        CABLES_37B0.contains(&pid)
    }
}

pub fn poll_lamzu(ctx: &mut Ctx) {
    for vendor in [0x373E, 0x37B0] {
        for (vid, pid) in ctx.products(vendor) {
            for c in ctx.find(vid, pid, |c| c.feat_len == 65) {
                let Some(dev) = Dev::open_feature_only(&c) else {
                    continue;
                };
                if !dev.set_feature(&power83_request()) {
                    continue;
                }
                sleep(Duration::from_millis(100));
                let reply = dev.get_feature(0x00).filter(|r| r.len() >= 10);
                let Some(Power83::Level { percent, charging }) =
                    reply.as_deref().and_then(parse_power83)
                else {
                    continue;
                };
                let cable = is_cable(vid, pid);
                let b = Battery {
                    key: ctx.stable_key(&format!("lamzu-{vid:04X}:{pid:04X}")),
                    name: model_name(vid, pid).into(),
                    via: if cable {
                        "USB cable"
                    } else {
                        "2.4 GHz receiver"
                    }
                    .into(),
                    kind: Kind::Mouse,
                    // 0 through a dongle: mouse off or out of range
                    connected: percent > 0 || cable,
                    percent: (percent > 0).then_some(percent),
                    charging,
                    ..Default::default()
                };
                ctx.add(b, vid, pid);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_and_cables() {
        assert_eq!(model_name(0x37B0, 0x0032), "LAMZU Thorn V2");
        assert_eq!(model_name(0x373E, 0x0016), "LAMZU Maya X V2");
        assert_eq!(model_name(0x37B0, 0x7777), "LAMZU wireless mouse");
        assert!(is_cable(0x373E, 0x001C) && !is_cable(0x373E, 0x001D));
        assert!(is_cable(0x37B0, 0x0056) && !is_cable(0x37B0, 0x000B));
    }
}
