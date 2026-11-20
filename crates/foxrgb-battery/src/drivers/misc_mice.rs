use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::util::clamp100;
use crate::{Battery, Ctx, Kind};

mod ajazz;
mod pixart;
mod xenta;

fn mouse(key: String, name: &str, via: &str) -> Battery {
    Battery {
        key,
        name: name.into(),
        via: via.into(),
        kind: Kind::Mouse,
        ..Default::default()
    }
}

fn via(cable: bool) -> &'static str {
    if cable {
        "USB cable"
    } else {
        "2.4 GHz receiver"
    }
}

fn sum(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |a, &v| a.wrapping_add(v))
}

/// Bit 7 charging, low 7 bits percent.
fn level_byte(b: u8) -> (u8, bool) {
    (b & 0x7F, b & 0x80 != 0)
}

// --- Motospeed X6 (Realtek 0BDA; receivers FFE0 / FFF2, cable FFF1) ---

const MOTOSPEED: u16 = 0x0BDA;
const MOTOSPEED_CABLE: u16 = 0xFFF1;

/// Report B4 answering B5 06: byte 20 is the level, plus 128 while charging.
fn motospeed_level(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() <= 20 || r[0] != 0xB4 {
        return None;
    }
    let raw = r[20];
    Some(if raw >= 128 {
        (clamp100(i32::from(raw) - 128), true)
    } else {
        (clamp100(raw.into()), false)
    })
}

fn poll_motospeed(ctx: &mut Ctx) {
    for pid in [0xFFE0, 0xFFF2, MOTOSPEED_CABLE] {
        if ctx.is_claimed(MOTOSPEED, pid) {
            continue;
        }
        let cable = pid == MOTOSPEED_CABLE;
        for coll in ctx.find(MOTOSPEED, pid, |c| c.iface == 4 && c.out_len >= 21) {
            let mut b = mouse(
                ctx.stable_key(&format!("motospeed-{MOTOSPEED:04X}:{pid:04X}")),
                "Motospeed X6",
                via(cable),
            );
            // The cable reports no level: it is plugged in, so it charges.
            if cable {
                b.charging = true;
                ctx.add(b, MOTOSPEED, pid);
                break;
            }
            let Some(dev) = Dev::open(&coll) else {
                continue;
            };
            dev.flush();
            if !dev.write(&[0xB5, 0x06]) {
                continue;
            }
            let Some((pct, charging)) =
                dev.read_until(Duration::from_millis(1500), motospeed_level)
            else {
                continue;
            };
            b.percent = Some(pct);
            b.charging = charging;
            ctx.add(b, MOTOSPEED, pid);
            break;
        }
    }
}

// --- Delux M800 Pro (248A; cable 5B2E, receiver 5B2F) ---

const DELUX: u16 = 0x248A;
const M800_CABLE: u16 = 0x5B2E;
const M800_RECEIVER: u16 = 0x5B2F;

static M800_SEQ: AtomicU8 = AtomicU8::new(0);

fn m800_request(len: usize, seq: u8) -> Vec<u8> {
    let mut r = vec![0u8; len.max(33)];
    r[..5].copy_from_slice(&[0x0C, 0x01, 0x20, 0x00, seq]);
    r
}

fn m800_valid(r: &[u8]) -> bool {
    r.len() >= 20 && r[0] == 0x0C && r[2] == 0x20
}

#[derive(Debug, PartialEq)]
struct M800 {
    connected: bool,
    percent: Option<u8>,
    charging: bool,
    full: bool,
}

/// [6] link, [18] percent, [19] state: 0 battery, 1 charging, 2 full.
fn m800_status(r: &[u8]) -> Option<M800> {
    let (pct, state) = (r[18], r[19]);
    if pct > 100 {
        return None;
    }
    let connected = r[6] != 0 || pct > 0;
    let known = (connected && pct > 0).then_some(pct);
    Some(match state {
        2 => M800 {
            connected,
            percent: Some(100),
            charging: true,
            full: true,
        },
        1 => M800 {
            connected,
            percent: known,
            charging: true,
            full: false,
        },
        _ => M800 {
            connected,
            percent: known,
            charging: false,
            full: false,
        },
    })
}

fn m800_query(dev: &Dev, len: usize) -> Option<Vec<u8>> {
    let seq = M800_SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if !dev.set_feature(&m800_request(len, seq)) {
        return None;
    }
    sleep(Duration::from_millis(20));
    dev.get_feature(0x0C).filter(|r| m800_valid(r))
}

fn poll_delux_m800(ctx: &mut Ctx) {
    for pid in [M800_CABLE, M800_RECEIVER] {
        if ctx.is_claimed(DELUX, pid) {
            continue;
        }
        let receiver = pid == M800_RECEIVER;
        for coll in ctx.find(DELUX, pid, |c| {
            c.page == 0xFF00 && c.usage == 1 && c.feat_len >= 33
        }) {
            let Some(dev) = Dev::open_feature_only(&coll) else {
                continue;
            };
            let len = coll.feat_len as usize;
            let Some(mut r) = m800_query(&dev, len) else {
                continue;
            };
            // A receiver that has not heard from the mouse yet often answers the second query.
            if receiver
                && r[6] == 0
                && let Some(again) = m800_query(&dev, len)
            {
                r = again;
            }
            let Some(st) = m800_status(&r) else { continue };
            let mut b = mouse(
                ctx.stable_key("delux-m800"),
                "Delux M800 Pro",
                via(!receiver),
            );
            b.connected = st.connected;
            b.percent = st.percent;
            b.charging = st.charging;
            b.full = st.full;
            if !receiver && st.connected {
                ctx.claim(DELUX, M800_RECEIVER);
            }
            ctx.add(b, DELUX, pid);
            break;
        }
    }
}

pub fn poll_misc_mice(ctx: &mut Ctx) {
    poll_motospeed(ctx);
    xenta::poll(ctx);
    poll_delux_m800(ctx);
    ajazz::poll_aj179(ctx);
    ajazz::poll_aj_series(ctx);
    ajazz::poll_aj139(ctx);
    // Same ids as Incott, different commands: the W1 claims its receiver first.
    pixart::poll_wraith_w1(ctx);
    pixart::poll_incott(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motospeed_charging_flag() {
        let mut r = vec![0u8; 32];
        r[0] = 0xB4;
        r[20] = 128 + 45;
        assert_eq!(motospeed_level(&r), Some((45, true)));
        r[20] = 80;
        assert_eq!(motospeed_level(&r), Some((80, false)));
        r[0] = 0xB5;
        assert_eq!(motospeed_level(&r), None);
    }

    #[test]
    fn m800_states() {
        let req = m800_request(33, 7);
        assert_eq!(&req[..5], &[0x0C, 0x01, 0x20, 0x00, 7]);
        let mut r = vec![0u8; 33];
        r[..3].copy_from_slice(&[0x0C, 0x01, 0x20]);
        r[8..12].copy_from_slice(b"M802");
        r[6] = 1;
        r[18] = 64;
        assert!(m800_valid(&r));
        assert_eq!(
            m800_status(&r),
            Some(M800 {
                connected: true,
                percent: Some(64),
                charging: false,
                full: false
            })
        );
        r[19] = 2;
        assert_eq!(m800_status(&r).unwrap().percent, Some(100));
        r[6] = 0;
        r[18] = 0;
        r[19] = 0;
        assert_eq!(
            m800_status(&r),
            Some(M800 {
                connected: false,
                percent: None,
                charging: false,
                full: false
            })
        );
    }
}
