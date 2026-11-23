use std::time::Duration;

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1E71;
const RECEIVER: u16 = 0x2101;
const CABLE: u16 = 0x2129;

#[derive(Debug, PartialEq)]
struct Telemetry {
    percent: u8,
    mv: u32,
    charging: bool,
}

/// Telemetry `4E 02 97 ..`: `[8..10]` cell mV big-endian, `[17..19]` percent little-endian, `[22]` bit 0 on the cable.
/// `Some(None)` for a telemetry frame without a usable percent.
fn parse_frame(r: &[u8]) -> Option<Option<Telemetry>> {
    if r.len() < 27 || r[0] != 0x4E || r[1] != 0x02 || r[2] != 0x97 {
        return None;
    }
    let level = u16::from_le_bytes([r[17], r[18]]);
    Some((level <= 100).then(|| Telemetry {
        percent: level as u8,
        mv: u16::from_be_bytes([r[8], r[9]]) as u32,
        charging: r[22] & 0x01 != 0,
    }))
}

fn read_telemetry(c: &Coll) -> Option<Telemetry> {
    let dev = Dev::open(c)?;
    dev.flush();
    // Report id 0, then: the mouse (02), read a property (81), telemetry (B0).
    if !dev.write(&[0x00, 0x4E, 0x02, 0x81, 0x00, 0xB0]) {
        return None;
    }
    // Acks (4E E5) and other traffic come first.
    for _ in 0..10 {
        if let Some(t) = parse_frame(&dev.read(Duration::from_millis(150))?) {
            return t;
        }
    }
    None
}

pub fn poll_nzxt(ctx: &mut Ctx) {
    let mut present = None;
    let mut reading = None;
    for pid in [CABLE, RECEIVER] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page == 0xFFCA && c.usage == 0x0001);
        if colls.is_empty() {
            continue;
        }
        present.get_or_insert(pid);
        ctx.claim(VID, pid);
        if reading.is_none() {
            reading = colls.iter().find_map(read_telemetry).map(|t| (pid, t));
        }
    }
    let Some(pid) = reading.as_ref().map(|r| r.0).or(present) else {
        return;
    };
    let mut b = Battery {
        key: ctx.stable_key("nzxt-lift-elite"),
        name: "NZXT Lift Elite".into(),
        via: if pid == CABLE {
            "USB cable"
        } else {
            "2.4 GHz receiver"
        }
        .into(),
        kind: Kind::Mouse,
        grace_secs: 1800,
        ..Default::default()
    };
    match reading {
        Some((_, t)) => {
            b.percent = Some(t.percent);
            b.charging = t.charging;
        }
        // Mouse off, or the vendor app holds the dongle.
        None => b.asleep = true,
    }
    ctx.add(b, VID, pid);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_frame() {
        let mut r = vec![0u8; 64];
        r[..3].copy_from_slice(&[0x4E, 0x02, 0x97]);
        r[8..10].copy_from_slice(&[0x0F, 0xA0]);
        r[17] = 81;
        r[22] = 0x01;
        assert_eq!(
            parse_frame(&r),
            Some(Some(Telemetry {
                percent: 81,
                mv: 4000,
                charging: true
            }))
        );
        r[18] = 1;
        assert_eq!(parse_frame(&r), Some(None));
        r[1] = 0xE5;
        assert_eq!(parse_frame(&r), None);
    }
}
