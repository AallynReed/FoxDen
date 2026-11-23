use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x361D;

const VOLTAGE: u8 = 5;
const LINK: u8 = 36;
const CHARGING: u8 = 37;
const CHARGE_STATE: u8 = 38;
const WAKE_ALL: u8 = 96;

struct Dongle {
    pid: u16,
    name: &'static str,
    starlight: bool,
    out_id: u8,
    in_id: u8,
    /// The read the vendor configurator sends before wake/all.
    first_get: u8,
}

const DONGLES: [Dongle; 3] = [
    Dongle {
        pid: 0x0100,
        name: "Finalmouse UltralightX",
        starlight: false,
        out_id: 0x04,
        in_id: 0x05,
        first_get: 21,
    },
    Dongle {
        pid: 0x0300,
        name: "Finalmouse Starlight X",
        starlight: true,
        out_id: 0x01,
        in_id: 0x02,
        first_get: 57,
    },
    Dongle {
        pid: 0x0301,
        name: "Finalmouse Starlight X",
        starlight: true,
        out_id: 0x01,
        in_id: 0x02,
        first_get: 57,
    },
];

const CURVE_MV: [i32; 7] = [3000, 3620, 3660, 3740, 3880, 4170, 4380];
const CURVE_PCT: [f64; 7] = [0.2, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0];

fn percent_from_mv(mv: i32) -> u8 {
    if mv <= CURVE_MV[0] {
        return 0;
    }
    let Some(i) = CURVE_MV.iter().position(|&p| mv <= p) else {
        return 100;
    };
    let f = (mv - CURVE_MV[i - 1]) as f64 / (CURVE_MV[i] - CURVE_MV[i - 1]) as f64;
    (CURVE_PCT[i - 1] + (CURVE_PCT[i] - CURVE_PCT[i - 1]) * f).round() as u8
}

#[derive(Default, Debug, PartialEq)]
struct Burst {
    percent: Option<u8>,
    mv: u32,
    charging: Option<bool>,
    linked: Option<bool>,
}

impl Burst {
    /// One input report `[in id][2 + n][command][n][payload]`.
    fn take(&mut self, r: &[u8], in_id: u8) {
        if r.len() < 4 || r[0] != in_id {
            return;
        }
        let inner = (r[1] as usize).min(r.len() - 2);
        if inner < 2 {
            return;
        }
        let n = (r[3] as usize).min(inner - 2);
        let p = &r[4..4 + n];
        let mv = |lo: usize| {
            let v = u16::from_le_bytes([p[lo], p[lo + 1]]) as u32;
            (1..5000).contains(&v).then_some(v)
        };
        match r[2] {
            VOLTAGE if n >= 2 => self.mv = mv(0).unwrap_or(self.mv),
            CHARGE_STATE if n >= 3 && p[0] <= 100 => {
                self.percent = Some(p[0]);
                self.mv = mv(1).unwrap_or(self.mv);
            }
            CHARGING if n >= 1 => self.charging = Some(p[0] == 1),
            LINK if n >= 1 => self.linked = Some(p[0] != 0),
            _ => {}
        }
    }

    fn has_level(&self, starlight: bool) -> bool {
        self.percent.is_some() || (!starlight && self.mv > 0)
    }
}

/// Two GETs (empty payload), then the dongle pushes its status burst.
fn query(dev: &Dev, d: &Dongle) -> Option<Burst> {
    for cmd in [d.first_get, WAKE_ALL] {
        if !dev.write(&[d.out_id, 2, 0x80 | cmd, 0]) {
            return None;
        }
    }
    let mut st = Burst::default();
    let mut end = Instant::now() + Duration::from_millis(1000);
    let mut shortened = false;
    loop {
        let level = st.has_level(d.starlight);
        if level && st.charging.is_some() {
            break;
        }
        // The charging state normally follows the level within the same burst.
        if level && !shortened {
            end = end.min(Instant::now() + Duration::from_millis(150));
            shortened = true;
        }
        let Some(left) = end.checked_duration_since(Instant::now()) else {
            break;
        };
        let Some(r) = dev.read(left) else { break };
        st.take(&r, d.in_id);
    }
    Some(st)
}

pub fn poll_finalmouse(ctx: &mut Ctx) {
    for d in &DONGLES {
        if ctx.is_claimed(VID, d.pid) {
            continue;
        }
        let mut colls = ctx.find(VID, d.pid, |c| {
            c.page == 0xFF00 && (d.starlight || c.usage == 1) && c.in_len >= 4 && c.out_len >= 4
        });
        colls.sort_by_key(|c| c.usage != 1);
        for c in colls {
            let Some(dev) = Dev::open(&c) else { continue };
            let Some(st) = query(&dev, d) else { continue };
            let percent = st
                .percent
                .or((st.mv > 0).then(|| percent_from_mv(st.mv as i32)));
            if percent.is_none() && st.linked != Some(false) {
                continue;
            }
            let b = Battery {
                key: ctx.stable_key(&format!("finalmouse-361D:{:04X}", d.pid)),
                name: d.name.into(),
                via: "2.4 GHz receiver".into(),
                kind: Kind::Mouse,
                percent,
                // no level but no link either: the mouse sleeps or is off
                asleep: percent.is_none(),
                charging: st.charging == Some(true),
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, VID, d.pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voltage_curve() {
        assert_eq!(percent_from_mv(2900), 0);
        assert_eq!(percent_from_mv(3880), 50);
        assert_eq!(percent_from_mv(3810), 38);
        assert_eq!(percent_from_mv(4500), 100);
    }

    #[test]
    fn burst_frames() {
        let mut st = Burst::default();
        st.take(&[0x05, 5, CHARGE_STATE, 3, 77, 0x10, 0x0F, 0, 0], 0x05);
        st.take(&[0x05, 3, CHARGING, 1, 1, 0, 0], 0x05);
        st.take(&[0x02, 3, LINK, 1, 0], 0x05);
        assert_eq!(
            st,
            Burst {
                percent: Some(77),
                mv: 3856,
                charging: Some(true),
                linked: None
            }
        );
    }
}
