//! PixArt 093A:522C (dongle) / 622C (cable): shared by Wraith W1 and Incott mice, which use different commands.

use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::{level_byte, mouse, via};
use crate::Ctx;
use crate::hid::{Coll, Dev};

const VID: u16 = 0x093A;
const DONGLE: u16 = 0x522C;
const CABLE: u16 = 0x622C;
const REPORT: u8 = 0x09;
const W1_INFO: u8 = 0x8F;
const W1_BUTTON: u8 = 0x86;
const INCOTT_STATUS: u8 = 0x89;

fn feature_colls(ctx: &Ctx, pid: u16) -> Vec<Coll> {
    let mut v = ctx.find(VID, pid, |c| c.page == 0xFF05 && c.feat_len >= 9);
    // On the cable the first FF05 collection rejects SET_FEATURE.
    v.reverse();
    v
}

/// Device info reply: byte 3 is the model, 18 W1, 20 W1 Freestyle; any other code is not a W1.
fn w1_model(r: &[u8]) -> Option<Option<u8>> {
    (r.len() >= 4 && r[0] == REPORT && r[1] == W1_INFO)
        .then(|| matches!(r[3], 18 | 20).then_some(r[3]))
}

/// Pushed status input report 09: battery byte right after the id.
fn w1_status(r: &[u8]) -> Option<(u8, bool)> {
    (r.len() >= 2 && r[0] == REPORT && r[1] & 0x7F <= 100).then(|| level_byte(r[1]))
}

fn incott_status(r: &[u8]) -> Option<(u8, bool)> {
    let ok = r.len() >= 4 && r[0] == REPORT && r[1] == INCOTT_STATUS && (r[2] | r[3]) != 0;
    ok.then(|| level_byte(r[2])).filter(|s| s.0 <= 100)
}

/// Last W1 model seen per id, so a sleeping W1 is never handed to the Incott query.
static W1_SEEN: Mutex<[u8; 2]> = Mutex::new([0; 2]);

enum Probe {
    W1(Dev, u8),
    Other,
    Silent,
}

fn probe_w1(colls: &[Coll]) -> Probe {
    for c in colls {
        let Some(dev) = Dev::open_feature_only(c) else {
            continue;
        };
        if !dev.set_feature(&[REPORT, W1_INFO]) {
            continue;
        }
        for _ in 0..10 {
            sleep(Duration::from_millis(50));
            let Some(r) = dev.get_feature(REPORT) else {
                break;
            };
            match w1_model(&r) {
                Some(Some(m)) => return Probe::W1(dev, m),
                Some(None) => return Probe::Other,
                None => {}
            }
        }
    }
    Probe::Silent
}

/// One per-button read, timed like the vendor driver; the mouse pushes its status in answer.
fn read_button(dev: &Dev, button: u8) -> bool {
    if !dev.set_feature(&[REPORT, W1_BUTTON, button]) {
        return false;
    }
    for wait in [150, 20] {
        sleep(Duration::from_millis(wait));
        match dev.get_feature(REPORT) {
            Some(f) if f.len() >= 4 && f[1] == W1_BUTTON => break,
            Some(_) => {}
            None => break,
        }
    }
    true
}

fn listen(inputs: &[Dev], for_ms: u64) -> Option<(u8, bool)> {
    let end = Instant::now() + Duration::from_millis(for_ms);
    while Instant::now() < end {
        for d in inputs {
            if let Some(s) = d
                .read(Duration::from_millis(50))
                .as_deref()
                .and_then(w1_status)
            {
                return Some(s);
            }
        }
    }
    None
}

/// Button sweep 0, 1, 2, 4, 3, then a 700 ms keep-alive read while the status push is awaited.
fn w1_battery(ctx: &Ctx, pid: u16, dev: &Dev) -> Option<(u8, bool)> {
    let inputs: Vec<Dev> = ctx
        .find(VID, pid, |c| c.page == 0xFF05 && c.in_len >= 2)
        .iter()
        .filter_map(Dev::open_read_only)
        .inspect(|d| d.flush())
        .collect();
    if inputs.is_empty() {
        return None;
    }
    sleep(Duration::from_millis(200));
    let mut sent = false;
    for b in [0, 1, 2, 4, 3] {
        sent |= read_button(dev, b);
    }
    if !sent {
        return None;
    }
    if let Some(s) = listen(&inputs, 400) {
        return Some(s);
    }
    for _ in 0..4 {
        if !read_button(dev, 0) {
            break;
        }
        if let Some(s) = listen(&inputs, 550) {
            return Some(s);
        }
    }
    None
}

pub(super) fn poll_wraith_w1(ctx: &mut Ctx) {
    let mut seen = W1_SEEN.lock().unwrap_or_else(|e| e.into_inner());
    for pid in [DONGLE, CABLE] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = feature_colls(ctx, pid);
        if colls.is_empty() {
            continue;
        }
        let known = &mut seen[usize::from(pid == CABLE)];
        let dev = match probe_w1(&colls) {
            Probe::Other => {
                *known = 0;
                continue;
            }
            Probe::W1(dev, model) => {
                *known = model;
                Some(dev)
            }
            Probe::Silent if *known != 0 => None,
            Probe::Silent => continue,
        };
        let cable = pid == CABLE;
        let name = if *known == 20 {
            "Wraith W1 Freestyle"
        } else {
            "Wraith W1"
        };
        let mut b = mouse(
            ctx.stable_key(&format!("wraith-w1-{VID:04X}:{pid:04X}")),
            name,
            via(cable),
        );
        if !cable {
            b.grace_secs = 1800;
        }
        b.connected = false;
        if let Some((pct, charging)) = dev.as_ref().and_then(|d| w1_battery(ctx, pid, d)) {
            b.charging = charging;
            // 0 from the dongle: the mouse is off.
            b.connected = cable || pct > 0 || charging;
            b.percent = b.connected.then_some(pct);
        }
        // Claimed even when silent, so the Incott query never reaches a W1.
        ctx.add(b, VID, pid);
    }
}

pub(super) fn poll_incott(ctx: &mut Ctx) {
    for pid in [DONGLE, CABLE] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for coll in feature_colls(ctx, pid) {
            let Some(dev) = Dev::open_feature_only(&coll) else {
                continue;
            };
            if !dev.set_feature(&[REPORT, INCOTT_STATUS]) {
                continue;
            }
            let mut status = None;
            for _ in 0..10 {
                sleep(Duration::from_millis(50));
                let Some(r) = dev.get_feature(REPORT) else {
                    break;
                };
                // A reply with bytes 2 and 3 both 0 is stale.
                status = incott_status(&r);
                if status.is_some() {
                    break;
                }
            }
            let Some((pct, charging)) = status else {
                continue;
            };
            let cable = pid == CABLE;
            // The ids are shared by the G23 V2 and the G24.
            let mut b = mouse(
                ctx.stable_key(&format!("incott-{VID:04X}:{pid:04X}")),
                "Incott mouse",
                via(cable),
            );
            b.charging = charging;
            b.connected = cable || pct > 0;
            b.percent = b.connected.then_some(pct);
            if !cable {
                b.grace_secs = 1800;
            }
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w1_model_codes() {
        assert_eq!(
            w1_model(&[0x09, 0x8F, 0, 20, 0, 0, 0, 0, 0]),
            Some(Some(20))
        );
        assert_eq!(w1_model(&[0x09, 0x8F, 0, 6, 0, 0, 0, 0, 0]), Some(None));
        assert_eq!(w1_model(&[0x09, 0x86, 0, 20, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn status_bytes_carry_the_charging_bit() {
        assert_eq!(w1_status(&[0x09, 0x80 | 64, 0x11]), Some((64, true)));
        assert_eq!(w1_status(&[0x09, 0x7F]), None);
        assert_eq!(
            incott_status(&[0x09, 0x89, 0x80 | 30, 0x23, 0, 0, 0, 0, 0]),
            Some((30, true))
        );
        assert_eq!(
            incott_status(&[0x09, 0x89, 77, 0x23, 0, 0, 0, 0, 0]),
            Some((77, false))
        );
        assert_eq!(incott_status(&[0x09, 0x89, 0, 0, 0, 0, 0, 0, 0]), None);
    }
}
