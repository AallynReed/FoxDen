//! 8BitDo controllers: read-only status queries on the vendor channel (FF7A/FFA0, report 81 out,
//! 02 in), else the level byte in D-input / Bluetooth gamepad input reports.

use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x2DC8;
/// Internal ids of the 2.4 GHz dongle, busy and idle.
const DONGLE: u16 = 0x6013;
const DONGLE_IDLE: u16 = 0x3109;

const LINK: [u8; 4] = [0x81, 0x04, 0x20, 0x01];
const LEVEL: [u8; 7] = [0x81, 0x04, 0x21, 0x01, 0x00, 0x00, 0x06];
const INFO: [u8; 4] = [0x81, 0x04, 0x00, 0x01];

/// (pid, name, reads feature report 06 before its input reports)
const MODELS: [(u16, &str, bool); 19] = [
    (0x310B, "8BitDo Ultimate 2 Wireless", false),
    (0x6012, "8BitDo Ultimate 2", false),
    (0x6013, "8BitDo Ultimate 2 Wireless", false),
    (0x202F, "8BitDo Ultimate 3-mode", false),
    (0x202E, "8BitDo Ultimate 3-mode", false),
    (0x301B, "8BitDo Ultimate 2C Wireless", false),
    (0x301A, "8BitDo Ultimate 2C Wireless", false),
    (0x310A, "8BitDo Ultimate 2C Wireless", false),
    (0x3012, "8BitDo Ultimate 2.4G", false),
    (0x3013, "8BitDo Ultimate 2.4G", false),
    (0x3109, "8BitDo Ultimate 2.4G", false),
    (0x6007, "8BitDo Ultimate Bluetooth", false),
    (0x6009, "8BitDo Pro 3", true),
    (0x6003, "8BitDo Pro 2", true),
    (0x6006, "8BitDo Pro 2", true),
    (0x6001, "8BitDo SN30 Pro", true),
    (0x6101, "8BitDo SN30 Pro", true),
    (0x6000, "8BitDo SF30 Pro", true),
    (0x6100, "8BitDo SF30 Pro", true),
];

/// Last level of the Ultimate 2, kept while it sits on its dock with the radio off.
static LAST_ULTIMATE2: AtomicI32 = AtomicI32::new(-1);

fn model(pid: u16) -> Option<&'static (u16, &'static str, bool)> {
    MODELS.iter().find(|m| m.0 == pid)
}

fn ultimate2(pid: u16) -> bool {
    matches!(pid, 0x310B | 0x6012 | 0x6013)
}

fn last_ultimate2() -> Option<u8> {
    u8::try_from(LAST_ULTIMATE2.load(Ordering::Relaxed)).ok()
}

fn model_name(c: &Coll) -> String {
    if let Some(m) = model(c.pid) {
        return m.1.into();
    }
    match c.product.as_str() {
        "" => "8BitDo Wireless Controller".into(),
        p if p.to_lowercase().starts_with("8bitdo") => p.into(),
        p => format!("8BitDo {p}"),
    }
}

fn base(c: &Coll, ctx: &Ctx) -> Battery {
    let key = if ultimate2(c.pid) {
        "8bitdo-ultimate2".to_string()
    } else {
        format!("8bitdo-{VID:04x}:{:04x}", c.pid)
    };
    Battery {
        key: ctx.stable_key(&key),
        name: model_name(c),
        kind: Kind::Gamepad,
        ..Default::default()
    }
}

/// A vendor reply `02 04 .. .. <lo> <hi>` echoing the request's command; needs byte 18.
fn is_reply(r: &[u8], req: &[u8]) -> bool {
    r.len() >= 19 && r[0] == 0x02 && r[1] == 0x04 && r[4] == req[2] && r[5] == req[3]
}

/// Level byte of the level reply: bits 0-6 percent (1..=100), bit 7 charging.
fn vendor_level(r: &[u8]) -> Option<(u8, bool)> {
    let pct = r[18] & 0x7F;
    (1..=100)
        .contains(&pct)
        .then_some((pct, r[18] & 0x80 != 0 && pct < 100))
}

fn internal_pid(r: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*r.get(22)?, *r.get(23)?]))
}

fn ask(d: &Dev, req: &[u8]) -> Option<Vec<u8>> {
    d.flush();
    if !d.write(req) {
        return None;
    }
    d.read_until(Duration::from_millis(300), |r| {
        is_reply(r, req).then(|| r.to_vec())
    })
}

fn poll_vendor(c: &Coll, ctx: &Ctx) -> Option<Battery> {
    let d = Dev::open(c)?;
    let ult2 = ultimate2(c.pid);
    let mut b = Battery {
        via: "2.4 GHz receiver".into(),
        grace_secs: 1800,
        ..base(c, ctx)
    };
    let linked = ask(&d, &LINK).is_none_or(|r| r[18] != 0);
    if linked && let Some((pct, chg)) = ask(&d, &LEVEL).as_deref().and_then(vendor_level) {
        b.percent = Some(pct);
        b.charging = chg;
        if ult2 {
            LAST_ULTIMATE2.store(pct as i32, Ordering::Relaxed);
        }
        return Some(b);
    }
    let docked = |mut b: Battery| {
        if ult2 && last_ultimate2().is_some() {
            b.asleep = true;
        } else {
            b.connected = false;
        }
        b
    };
    if let Some(r) = ask(&d, &INFO)
        && let Some(id) = internal_pid(&r)
    {
        if id != DONGLE && id != DONGLE_IDLE {
            b.via = "USB cable".into();
            b.charging = true;
            b.percent = if ult2 { last_ultimate2() } else { None };
            return Some(b);
        }
        return Some(docked(b));
    }
    matches!(c.pid, DONGLE | DONGLE_IDLE).then(|| docked(b))
}

/// Level byte 14 of gamepad input reports 01 / 04.
fn input_level(r: &[u8], min_len: usize) -> Option<(u8, bool)> {
    if r.len() < min_len || !matches!(r[0], 0x01 | 0x04) || r[14] & 0x7F > 100 {
        return None;
    }
    let pct = r[14] & 0x7F;
    Some((pct, r[14] & 0x80 != 0 && pct < 100))
}

fn poll_gamepad(c: &Coll, ctx: &Ctx) -> Option<Battery> {
    let wants_feature = model(c.pid).is_some_and(|m| m.2) && c.feat_len > 0;
    let d = match wants_feature.then(|| Dev::open(c)).flatten() {
        Some(d) => {
            let _ = d.get_feature(0x06);
            d
        }
        None => Dev::open_read_only(c)?,
    };
    // Older Ultimate 2 firmware sends 12-byte reports without the level.
    let min_len = if c.pid == 0x6012 { 34 } else { 15 };
    let end = Instant::now() + Duration::from_millis(600);
    let (pct, chg) = loop {
        let left = end.checked_duration_since(Instant::now())?;
        if let Some(v) = d
            .read(left.min(Duration::from_millis(100)))
            .and_then(|r| input_level(&r, min_len))
        {
            break v;
        }
    };
    let bt = c.path.contains("00001124");
    if ultimate2(c.pid) && pct > 0 {
        LAST_ULTIMATE2.store(pct as i32, Ordering::Relaxed);
    }
    Some(Battery {
        via: if bt { "Bluetooth" } else { "2.4 GHz receiver" }.into(),
        percent: Some(pct),
        charging: chg,
        grace_secs: 180,
        ..base(c, ctx)
    })
}

pub fn poll_8_bit_do(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        let vendor = ctx.find(vid, pid, |c| {
            matches!(c.page, 0xFF7A | 0xFFA0)
                && c.usage == 0x0001
                && c.in_len >= 64
                && c.out_len >= 64
        });
        if let Some(b) = vendor.iter().find_map(|c| poll_vendor(c, ctx)) {
            ctx.add(b, vid, pid);
            continue;
        }
        if model(pid).is_none() {
            continue;
        }
        let pads = ctx.find(vid, pid, |c| {
            c.page == 0x01 && matches!(c.usage, 0x04 | 0x05) && c.in_len >= 15
        });
        if let Some(b) = pads.iter().find_map(|c| poll_gamepad(c, ctx)) {
            ctx.add(b, vid, pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vendor_reply(lo: u8, hi: u8, at: usize, v: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..6].copy_from_slice(&[0x02, 0x04, 0x04, 0x00, lo, hi]);
        r[at..at + v.len()].copy_from_slice(v);
        r
    }

    #[test]
    fn vendor_replies() {
        let r = vendor_reply(0x21, 0x01, 18, &[0x80 | 64]);
        assert!(is_reply(&r, &LEVEL) && !is_reply(&r, &LINK));
        assert_eq!(vendor_level(&r), Some((64, true)));
        assert_eq!(vendor_level(&vendor_reply(0x21, 0x01, 18, &[0])), None);
        assert_eq!(
            internal_pid(&vendor_reply(0x00, 0x01, 22, &[0x13, 0x60])),
            Some(DONGLE)
        );
    }

    #[test]
    fn gamepad_report_level() {
        let mut r = vec![0u8; 34];
        r[0] = 0x01;
        r[14] = 0x80 | 100;
        assert_eq!(input_level(&r, 34), Some((100, false)));
        r[14] = 55;
        assert_eq!(input_level(&r, 34), Some((55, false)));
        assert_eq!(input_level(&r[..20], 34), None);
        r[0] = 0x03;
        assert_eq!(input_level(&r, 15), None);
    }
}
