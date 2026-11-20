use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::vendor_frames::RACE_BATTERY;
use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3329;
/// (pid, on the cable, name); cables first so the direct link wins.
const MODELS: &[(u16, bool, &str)] = &[
    (0x4B1A, true, "Audeze Maxwell"),
    (0x4B1E, true, "Audeze Maxwell"),
    (0x4B19, false, "Audeze Maxwell"),
    (0x4B18, false, "Audeze Maxwell"),
    (0x4B29, false, "Audeze Maxwell 2"),
    (0x4B28, false, "Audeze Maxwell 2"),
];
/// A headset reads 0% for a while after power-on.
const POWER_ON: Duration = Duration::from_secs(90);

static FIRST_SEEN: Mutex<Vec<(u16, Instant)>> = Mutex::new(Vec::new());

/// Report 07 keeps recent replies; the battery shows up as `D6 0C 00 00 [percent]` somewhere in it.
fn find_level(r: &[u8]) -> Option<u8> {
    r.windows(5)
        .find(|w| w[..4] == [0xD6, 0x0C, 0x00, 0x00] && w[4] <= 100)
        .map(|w| w[4])
}

fn read_level(c: &Coll) -> Option<u8> {
    let dev = Dev::open(c)?;
    let mut level = None;
    for _ in 0..2 {
        if !dev.write(&RACE_BATTERY) {
            return None;
        }
        // Later frames are newer: read a few and keep the last level.
        for i in 0..4 {
            sleep(Duration::from_millis(60));
            if let Some(l) = dev.get_input_report(0x07).as_deref().and_then(find_level) {
                level = Some(l);
            }
            if i >= 2 && level.is_some() {
                return level;
            }
        }
        if level.is_some() {
            break;
        }
    }
    level
}

pub fn poll_audeze(ctx: &mut Ctx) {
    let mut seen = FIRST_SEEN.lock().unwrap_or_else(|e| e.into_inner());
    for &(pid, cable, name) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let Some(c) = ctx
            .find(VID, pid, |c| {
                c.page == 0xFF13 && c.usage == 0x0001 && c.in_len > 0
            })
            .into_iter()
            .next()
        else {
            seen.retain(|s| s.0 != pid);
            continue;
        };
        // With no headset linked the dongle renames itself and repeats a stale level.
        if !cable && c.product.to_lowercase().ends_with("dongle") {
            ctx.claim(VID, pid);
            seen.retain(|s| s.0 != pid);
            continue;
        }
        let level = read_level(&c);
        let now = Instant::now();
        let first = match seen.iter().find(|s| s.0 == pid) {
            Some(s) => s.1,
            None => {
                seen.push((pid, now));
                now
            }
        };
        let fresh = level.filter(|&l| l > 0 || now.duration_since(first) >= POWER_ON);
        let b = Battery {
            key: ctx.stable_key("audeze-maxwell"),
            name: name.into(),
            via: if cable {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Headset,
            percent: fresh,
            // No charging flag: on its cable it always charges.
            charging: fresh.is_some() && cable,
            asleep: fresh.is_none(),
            grace_secs: 300,
            ..Default::default()
        };
        ctx.add(b, VID, pid);
        for &(other, ..) in MODELS {
            ctx.claim(VID, other);
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_marker_in_the_rolling_buffer() {
        let mut r = vec![0u8; 62];
        r[0] = 0x07;
        r[20..25].copy_from_slice(&[0xD6, 0x0C, 0x00, 0x00, 0x47]);
        assert_eq!(find_level(&r), Some(71));
        r[24] = 0xC8;
        assert_eq!(find_level(&r), None);
        assert_eq!(
            RACE_BATTERY[..9],
            [0x06, 0x07, 0x80, 0x05, 0x5A, 0x03, 0x00, 0xD6, 0x0C]
        );
    }
}
