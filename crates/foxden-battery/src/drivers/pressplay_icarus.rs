use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x7768;
const ICARUS_V2: u16 = 0x6434;

/// Reply `[06][05][26] .. [13] percent`.
fn parse_reply(r: &[u8]) -> Option<u8> {
    (r.len() >= 14 && r[..3] == [0x06, 0x05, 0x26] && r[13] <= 100).then(|| r[13])
}

fn read_level(c: &Coll) -> Option<u8> {
    let dev = Dev::open(c)?;
    dev.flush();
    parse_reply(&dev.set_output_and_read(&[0x06, 0x05], Duration::from_millis(500))?)
}

pub fn poll_press_play_icarus(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, ICARUS_V2) {
        return;
    }
    let colls = ctx.find(VID, ICARUS_V2, |c| {
        c.page == 0xFFB2 && c.usage == 0x0001 && c.in_len == 64 && c.out_len == 64
    });
    for c in colls {
        let Some(pct) = read_level(&c) else { continue };
        let b = Battery {
            key: ctx.stable_key("pressplay-icarus-v2"),
            name: "Press Play ICARUS V2 Nearlink".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            percent: Some(pct),
            ..Default::default()
        };
        ctx.add(b, VID, ICARUS_V2);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_at_byte_13() {
        let mut r = vec![0u8; 64];
        r[..3].copy_from_slice(&[0x06, 0x05, 0x26]);
        r[13] = 0x64;
        assert_eq!(parse_reply(&r), Some(100));
        r[13] = 0x5A;
        assert_eq!(parse_reply(&r), Some(90));
        r[2] = 0x25;
        assert_eq!(parse_reply(&r), None);
    }
}
