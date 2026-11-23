use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::hid::Dev;
use crate::util::{hex4, interpolate};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x04A5;
const CW_CABLE: u16 = 0x8005;
const CW_RECEIVER: u16 = 0x8006;
const U2_RECEIVER: u16 = 0x8008;
const DW_RECEIVER: u16 = 0x800A;

/// Anchored on the mouse's own LED thresholds at 10, 25 and 50 %.
const CURVE: [(i32, i32); 6] = [
    (4200, 100),
    (4000, 75),
    (3890, 50),
    (3760, 25),
    (3680, 10),
    (3500, 0),
];

fn percent(mv: u32) -> u8 {
    interpolate(mv as i32, &CURVE)
}

/// EC-CW reply: (millivolts, reported percent). 0 mV means the mouse isn't linked.
fn parse_cw(r: &[u8]) -> Option<(u32, Option<u8>)> {
    let [lo, hi, pct, ..] = *r else { return None };
    let mv = u16::from_le_bytes([lo, hi]) as u32;
    if mv != 0 && !(3000..=4500).contains(&mv) {
        return None;
    }
    Some((mv, (1..=100).contains(&pct).then_some(pct)))
}

/// DW / U2 input report 09: millivolts, 0 when unlinked.
fn parse_dw(r: &[u8]) -> Option<u32> {
    if r.len() < 5 || r[0] != 0x09 || r[1] == 0xE6 || r[4] != 0xC8 {
        return None;
    }
    Some(u16::from_be_bytes([r[1], r[2]]) as u32 * 10)
}

/// Over radio only the voltage arrives, so charging on the dock is read from its trend.
#[derive(Default)]
struct Trend {
    started: bool,
    charging: bool,
    peak: u32,
    trough: u32,
}

impl Trend {
    fn update(&mut self, mv: u32) -> bool {
        if !self.started {
            self.started = true;
            self.charging = mv >= 4250;
            self.peak = mv;
            self.trough = mv;
        } else if self.charging {
            self.peak = self.peak.max(mv);
            if mv + 20 <= self.peak {
                self.charging = false;
                self.trough = mv;
            }
        } else {
            self.trough = self.trough.min(mv);
            if mv >= self.trough + 50 {
                self.charging = true;
                self.peak = mv;
            }
        }
        self.charging
    }
}

static TRENDS: Mutex<BTreeMap<u16, Trend>> = Mutex::new(BTreeMap::new());

/// One voltage sample; `reported` is the mouse's own percent when it sends one.
struct Sample {
    mv: u32,
    reported: Option<u8>,
    name: String,
}

fn sample_ec(ctx: &Ctx, pid: u16) -> Option<Sample> {
    let colls = ctx.find(VID, pid, |c| {
        (c.page >= 0xFF00 || c.page == 0x00FF) && c.out_len == 65
    });
    colls.iter().find_map(|c| {
        let dev = Dev::open(c)?;
        dev.flush();
        if !dev.write(&[0x00, 0xA5, 0x03]) {
            return None;
        }
        let (mv, reported) = parse_cw(&dev.read(Duration::from_secs(1))?)?;
        Some(Sample {
            mv,
            reported,
            name: "ZOWIE EC-CW".into(),
        })
    })
}

/// Commands go out on FF03 and answers come back on FF04.
fn sample_dw(ctx: &Ctx, pid: u16) -> Option<Sample> {
    let cmd = ctx
        .find(VID, pid, |c| c.page == 0xFF03 && c.out_len == 16)
        .into_iter()
        .next()?;
    let rep = ctx
        .find(VID, pid, |c| c.page == 0xFF04 && c.in_len == 16)
        .into_iter()
        .next()?;
    let input = Dev::open(&rep)?;
    let output = Dev::open(&cmd)?;
    input.flush();
    if !output.write(&[0x08, 0xB4, 0x15]) {
        return None;
    }
    let mv = input.read_until(Duration::from_millis(1500), |r| {
        (r.first() == Some(&0x09)).then(|| parse_dw(r))
    })??;
    let name = match cmd.product.as_str() {
        _ if pid == U2_RECEIVER => "ZOWIE U2".to_string(),
        p if p.is_empty() || p.contains("Receiver") => "ZOWIE DW mouse".into(),
        p => p.into(),
    };
    Some(Sample {
        mv,
        reported: None,
        name,
    })
}

pub fn poll_zowie(ctx: &mut Ctx) {
    let mut trends = TRENDS.lock().unwrap_or_else(|e| e.into_inner());
    for (_, pid) in ctx.products(VID) {
        let ec = matches!(pid, CW_CABLE | CW_RECEIVER);
        let sample = if ec {
            sample_ec(ctx, pid)
        } else {
            sample_dw(ctx, pid)
        };
        let Some(s) = sample else { continue };
        let cable = !matches!(pid, CW_RECEIVER | DW_RECEIVER | U2_RECEIVER);
        let mut b = Battery {
            key: ctx.stable_key(&format!("zowie-{}", hex4(pid))),
            name: s.name,
            via: if cable {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Mouse,
            charging: ec && cable,
            connected: s.mv != 0,
            ..Default::default()
        };
        if s.mv != 0 {
            b.percent = Some(s.reported.unwrap_or_else(|| percent(s.mv)));
            if !ec {
                b.charging = cable || trends.entry(pid).or_default().update(s.mv);
            }
        }
        ctx.add(b, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cw_voltage_and_percent() {
        assert_eq!(parse_cw(&[0x68, 0x10, 80]), Some((4200, Some(80))));
        assert_eq!(parse_cw(&[0, 0, 0]), Some((0, None)));
        assert_eq!(parse_cw(&[0x10, 0x27, 50]), None);
        assert_eq!(percent(3890), 50);
    }

    #[test]
    fn dw_voltage() {
        assert_eq!(parse_dw(&[0x09, 0x01, 0x7F, 0, 0xC8]), Some(3830));
        assert_eq!(parse_dw(&[0x09, 0xE6, 0x7F, 0, 0xC8]), None);
        assert_eq!(parse_dw(&[0x09, 0x01, 0x7F, 0, 0x00]), None);
    }

    #[test]
    fn trend_detects_dock() {
        let mut t = Trend::default();
        assert!(!t.update(3800));
        assert!(!t.update(3790));
        assert!(t.update(3845));
        assert!(t.update(3900));
        assert!(!t.update(3880));
    }
}
