use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::{guess_kind, hex4, interpolate};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x046D;
/// Lighting uses 0x07; vendor apps use their own.
const SW_ID: u8 = 0x0B;
const SHORT: u8 = 0x10;
const LONG: u8 = 0x11;
const DIRECT: u8 = 0xFF;

const UNIFIED_BATTERY: u16 = 0x1004;
const BATTERY_STATUS: u16 = 0x1000;
const BATTERY_VOLTAGE: u16 = 0x1001;
const ADC_MEASUREMENT: u16 = 0x1F20;
const DEVICE_INFO: u16 = 0x0003;
const DEVICE_NAME: u16 = 0x0005;

/// HID++ 2.0-only dongles that relay a single device on index 1.
fn passthrough(pid: u16) -> bool {
    pid == 0xC54F
}

/// Parameters of the reply to a request starting `head` ([report, index, b2, b3]), or the error code.
fn answer(head: [u8; 4], r: &[u8]) -> Option<Result<Vec<u8>, u8>> {
    if r.len() < 7 || !matches!(r[0], SHORT | LONG) || r[1] != head[1] {
        return None;
    }
    if r[2] == head[2] && r[3] == head[3] {
        return Some(Ok(r[4..].to_vec()));
    }
    if matches!(r[2], 0xFF | 0x8F) && r[3] == head[2] && r[4] == head[3] {
        return Some(Err(r[5]));
    }
    None
}

#[derive(Debug, Default, PartialEq)]
struct Reading {
    percent: Option<u8>,
    level: &'static str,
    charging: bool,
    full: bool,
    millivolts: u32,
}

/// 0x1004 get_status: state of charge, level flags, charging state.
fn unified(p: &[u8]) -> Option<Reading> {
    let [soc, flags, state, ..] = *p else {
        return None;
    };
    let mut r = Reading {
        charging: matches!(state, 1..=3),
        full: state == 3,
        ..Reading::default()
    };
    if (1..=100).contains(&soc) {
        r.percent = Some(soc);
    } else if flags != 0 {
        let (pct, text) = match flags {
            f if f & 8 != 0 => (100, "Full"),
            f if f & 4 != 0 => (60, "Good"),
            f if f & 2 != 0 => (20, "Low"),
            _ => (5, "Critical"),
        };
        r.percent = Some(pct);
        r.level = text;
    }
    Some(r)
}

/// 0x1000 get level and status.
fn status(p: &[u8]) -> Option<Reading> {
    let [level, _, state, ..] = *p else {
        return None;
    };
    let full = state == 3;
    let percent = match level {
        0 if full => Some(100),
        0..=100 => Some(level),
        _ => None,
    };
    Some(Reading {
        percent,
        charging: matches!(state, 1..=4),
        full,
        ..Reading::default()
    })
}

const LI_ION: &[(i32, i32)] = &[
    (4186, 100),
    (4067, 90),
    (3989, 80),
    (3922, 70),
    (3859, 60),
    (3811, 50),
    (3778, 40),
    (3751, 30),
    (3717, 20),
    (3671, 10),
    (3646, 5),
    (3579, 2),
    (3500, 0),
];

fn headset_curve(pid: u16) -> &'static [(i32, i32)] {
    match pid {
        0x0A66 => &[
            (4200, 100),
            (3850, 50),
            (3790, 30),
            (3750, 20),
            (3680, 5),
            (3330, 0),
        ],
        0x0AC4 => &[
            (4175, 100),
            (3817, 50),
            (3766, 30),
            (3730, 20),
            (3664, 5),
            (3310, 0),
        ],
        0x0A5C | 0x0A89 | 0x0A5B | 0x0A87 | 0x0AB5 | 0x0AFE => &[
            (4100, 100),
            (3950, 80),
            (3850, 60),
            (3750, 40),
            (3650, 20),
            (3500, 10),
            (3300, 5),
            (3150, 0),
        ],
        0x0AAA | 0x0ABA => &[
            (4102, 100),
            (4046, 95),
            (3993, 90),
            (3950, 85),
            (3915, 80),
            (3877, 75),
            (3838, 70),
            (3809, 65),
            (3789, 60),
            (3772, 55),
            (3758, 50),
            (3747, 45),
            (3736, 40),
            (3722, 35),
            (3707, 30),
            (3687, 25),
            (3661, 20),
            (3644, 15),
            (3548, 10),
            (3426, 5),
            (3293, 0),
        ],
        0x0AA7 | 0x0AFB => &[
            (4150, 100),
            (3830, 50),
            (3780, 30),
            (3740, 20),
            (3670, 5),
            (3320, 0),
        ],
        _ => &[
            (4100, 100),
            (3850, 50),
            (3780, 30),
            (3730, 20),
            (3650, 5),
            (3300, 0),
        ],
    }
}

/// 0x1001 get battery info and 0x1F20 ADC: big-endian millivolts, then flags.
fn voltage(p: &[u8], curve: &[(i32, i32)], charging: impl Fn(u8) -> bool) -> Option<Reading> {
    let [hi, lo, flags, ..] = *p else { return None };
    let mv = u16::from_be_bytes([hi, lo]);
    Some(Reading {
        percent: (mv != 0).then(|| interpolate(mv as i32, curve)),
        charging: charging(flags),
        millivolts: mv as u32,
        ..Reading::default()
    })
}

fn charging_10(state: u8) -> bool {
    (0x21..=0x26).contains(&state)
}

/// HID++ 1.0 register 0x0D: percent, next level, charge state.
fn register_percent(p: &[u8]) -> Option<Reading> {
    let [pct @ 1..=100, _, state, ..] = *p else {
        return None;
    };
    Some(Reading {
        percent: Some(pct),
        charging: charging_10(state),
        ..Reading::default()
    })
}

/// HID++ 1.0 register 0x07: level 1..=7, charge state.
fn register_level(p: &[u8]) -> Option<Reading> {
    let [level @ 1..=255, state, ..] = *p else {
        return None;
    };
    const PCT: [u8; 8] = [0, 5, 10, 20, 35, 50, 75, 100];
    let level = level.min(7);
    let text = match level {
        7 => "Full",
        5 | 6 => "Good",
        3 | 4 => "Low",
        _ => "Critical",
    };
    Some(Reading {
        percent: Some(PCT[level as usize]),
        level: text,
        charging: charging_10(state),
        ..Reading::default()
    })
}

fn kind_of_type(t: u8) -> Kind {
    match t {
        0x00 | 0x02 => Kind::Keyboard,
        0x03..=0x05 => Kind::Mouse,
        0x08 => Kind::Headset,
        0x0B | 0x0C => Kind::Gamepad,
        _ => Kind::Other,
    }
}

fn display_name(raw: &str) -> String {
    if ["Logitech", "G ", "MX"].iter().any(|p| raw.starts_with(p)) {
        raw.to_string()
    } else {
        format!("Logitech {raw}")
    }
}

/// 0x0003 get device info: unit id (same over every link) and model id, as hex.
fn identity(p: &[u8]) -> Option<(String, String)> {
    let unit = p.get(1..5)?;
    if unit.iter().all(|&b| b == 0) {
        return None;
    }
    let hex = |b: &[u8]| b.iter().map(|v| format!("{v:02X}")).collect::<String>();
    let model = p
        .get(7..13)
        .filter(|m| m.iter().any(|&b| b != 0))
        .map(hex)
        .unwrap_or_default();
    Some((hex(unit), model))
}

/// Groups the short and long collections of one USB interface.
fn interface_of(path: &str) -> String {
    let p = path.to_lowercase();
    let parts: Vec<&str> = p.split('#').collect();
    if parts.len() < 4 {
        return String::new();
    }
    let hw: Vec<&str> = parts[1]
        .split('&')
        .filter(|s| !s.starts_with("col"))
        .collect();
    let inst = parts[2].rsplit_once('&').map_or(parts[2], |(a, _)| a);
    format!("{}#{inst}", hw.join("&"))
}

fn is_bluetooth(path: &str) -> bool {
    let p = path.to_lowercase();
    p.contains("{00001812-") || p.contains("{00001124-")
}

struct Link {
    short: Option<Dev>,
    long: Option<Dev>,
}

impl Link {
    fn send(
        &self,
        idx: u8,
        b2: u8,
        b3: u8,
        params: &[u8],
        short: bool,
        wait: Duration,
    ) -> Option<Result<Vec<u8>, u8>> {
        let (dev, id, len) = match (&self.long, &self.short) {
            (Some(l), _) if !short => (l, LONG, 20),
            (_, Some(s)) => (s, SHORT, 7),
            _ => return None,
        };
        let head = [id, idx, b2, b3];
        let mut msg = vec![0u8; len];
        msg[..4].copy_from_slice(&head);
        let n = params.len().min(len - 4);
        msg[4..4 + n].copy_from_slice(&params[..n]);
        if !dev.write_timeout(&msg, Duration::from_millis(200)) {
            return None;
        }
        let end = Instant::now() + wait;
        while Instant::now() < end {
            for d in [&self.long, &self.short].into_iter().flatten() {
                if let Some(r) = d.read(Duration::from_millis(10))
                    && let Some(a) = answer(head, &r)
                {
                    return Some(a);
                }
            }
        }
        None
    }

    fn call(&self, idx: u8, feature: u8, function: u8, params: &[u8]) -> Option<Vec<u8>> {
        self.send(
            idx,
            feature,
            function << 4 | SW_ID,
            params,
            false,
            Duration::from_millis(500),
        )?
        .ok()
    }

    fn feature(&self, idx: u8, id: u16) -> Option<u8> {
        self.call(idx, 0x00, 0, &id.to_be_bytes())?
            .first()
            .copied()
            .filter(|&f| f != 0)
    }

    /// Root ping: protocol version, or the error code a HID++ 1.0 device or empty slot answers with.
    fn ping(&self, idx: u8) -> Option<Result<Vec<u8>, u8>> {
        self.send(
            idx,
            0x00,
            1 << 4 | SW_ID,
            &[0, 0, 0x5A],
            false,
            Duration::from_millis(400),
        )
    }

    fn register(&self, idx: u8, reg: u8) -> Option<Vec<u8>> {
        self.send(idx, 0x81, reg, &[], true, Duration::from_millis(500))?
            .ok()
    }

    fn battery_20(&self, idx: u8, pid: u16) -> Option<Reading> {
        if let Some(f) = self.feature(idx, UNIFIED_BATTERY)
            && let Some(r) = self.call(idx, f, 1, &[]).and_then(|p| unified(&p))
        {
            return Some(r);
        }
        if let Some(f) = self.feature(idx, BATTERY_STATUS)
            && let Some(r) = self.call(idx, f, 0, &[]).and_then(|p| status(&p))
        {
            return Some(r);
        }
        if let Some(f) = self.feature(idx, BATTERY_VOLTAGE)
            && let Some(r) = self
                .call(idx, f, 0, &[])
                .and_then(|p| voltage(&p, LI_ION, |b| b & 0x80 != 0))
        {
            return Some(r);
        }
        let f = self.feature(idx, ADC_MEASUREMENT)?;
        self.call(idx, f, 0, &[])
            .and_then(|p| voltage(&p, headset_curve(pid), |b| b == 0x03))
    }

    fn battery_10(&self, idx: u8) -> Option<Reading> {
        self.register(idx, 0x0D)
            .and_then(|p| register_percent(&p))
            .or_else(|| self.register(idx, 0x07).and_then(|p| register_level(&p)))
    }

    fn name_and_kind(&self, idx: u8) -> (Option<String>, Kind) {
        let Some(f) = self.feature(idx, DEVICE_NAME) else {
            return (None, Kind::Other);
        };
        let mut name = Vec::new();
        if let Some(&len) = self.call(idx, f, 0, &[]).as_deref().and_then(|p| p.first()) {
            while name.len() < len as usize {
                let Some(chunk) = self.call(idx, f, 1, &[name.len() as u8]) else {
                    break;
                };
                let before = name.len();
                name.extend(
                    chunk
                        .iter()
                        .take_while(|&&c| c != 0)
                        .take(len as usize - before),
                );
                if name.len() == before {
                    break;
                }
            }
        }
        let kind = self
            .call(idx, f, 2, &[])
            .and_then(|p| p.first().map(|&t| kind_of_type(t)));
        let name = String::from_utf8_lossy(&name).trim().to_string();
        (
            (!name.is_empty()).then(|| display_name(&name)),
            kind.unwrap_or_default(),
        )
    }

    fn identity(&self, idx: u8) -> Option<(String, String)> {
        let f = self.feature(idx, DEVICE_INFO)?;
        identity(&self.call(idx, f, 0, &[])?)
    }
}

/// One receiver, cable or Bluetooth link. Returns true when it reported or deduplicated a device.
fn poll_link(
    ctx: &mut Ctx,
    pid: u16,
    short: Option<&Coll>,
    long: Option<&Coll>,
    seen: &mut HashSet<String>,
) -> bool {
    let link = Link {
        short: short.and_then(Dev::open),
        long: long.and_then(Dev::open),
    };
    let Some(main) = long.or(short) else {
        return false;
    };
    if link.short.is_none() && link.long.is_none() {
        return false;
    }
    let receiver = main.product.contains("Receiver");
    let bluetooth = is_bluetooth(&main.path);
    let headset_page = long.is_some_and(|c| c.page == 0xFF43);
    let relay = passthrough(pid);

    let direct = |link: &Link| matches!(link.ping(DIRECT), Some(Ok(p)) if p.first().is_some_and(|&v| v >= 2));
    let indices: Vec<u8> = if relay {
        vec![1]
    } else if bluetooth || (!receiver && direct(&link)) {
        vec![DIRECT]
    } else {
        (1..=6).collect()
    };

    let mut any = false;
    for idx in indices {
        let (reading, name, kind, ident) = match link.ping(idx) {
            Some(Ok(p)) if p.first().is_some_and(|&v| v >= 2) => {
                let (name, kind) = link.name_and_kind(idx);
                let reading = link.battery_20(idx, pid);
                let ident = reading.as_ref().and_then(|_| link.identity(idx));
                (reading, name, kind, ident)
            }
            Some(Err(0x01)) => (link.battery_10(idx), None, Kind::Other, None),
            _ => continue,
        };
        let Some(r) = reading else { continue };
        any = true;
        if let Some((unit, _)) = &ident
            && !seen.insert(unit.clone())
        {
            ctx.claim(VID, pid);
            continue;
        }

        let name = name.unwrap_or_else(|| {
            if idx == DIRECT && !main.product.is_empty() {
                main.product.clone()
            } else {
                "Logitech wireless device".into()
            }
        });
        let mut kind = if relay { Kind::Mouse } else { kind };
        if kind == Kind::Other && headset_page {
            kind = Kind::Headset;
        }
        if kind == Kind::Other {
            kind = guess_kind(&name);
        }
        let via = if bluetooth {
            "Bluetooth"
        } else if idx == DIRECT && !headset_page {
            "USB cable"
        } else {
            "2.4 GHz receiver"
        };
        let base = match &ident {
            Some((_, model)) if !model.is_empty() => format!("logitech-{model}"),
            Some((unit, _)) => format!("logitech-{unit}"),
            None => format!("logitech-{}:{}-{idx}", hex4(VID), hex4(pid)),
        };
        let grace_secs = match (idx, &ident) {
            (DIRECT, Some(_)) => 60,
            (DIRECT, None) => 0,
            _ => 1800,
        };
        let b = Battery {
            key: ctx.stable_key(&base),
            name,
            via: via.into(),
            kind,
            percent: r.percent,
            level: r.level.into(),
            charging: r.charging,
            full: r.full,
            millivolts: r.millivolts,
            grace_secs,
            ..Battery::default()
        };
        ctx.add(b, VID, pid);
    }
    any
}

pub fn poll_logitech(ctx: &mut Ctx) {
    let mut seen = HashSet::new();
    for (vid, pid) in ctx.products(VID) {
        let relay = passthrough(pid);
        let shorts = ctx.find(vid, pid, |c| {
            c.out_len == 7
                && ((c.page == 0xFF00 && c.usage == 0x0001)
                    || (relay && c.page == 0xFF43 && c.usage == 0x0301))
        });
        let longs = ctx.find(vid, pid, |c| {
            c.out_len == 20
                && ((c.page == 0xFF00 && c.usage == 0x0002)
                    || (c.page == 0xFF43 && (c.usage == 0x0202 || (relay && c.usage == 0x0302))))
        });
        let mut groups: Vec<String> = Vec::new();
        for c in shorts.iter().chain(&longs) {
            let g = interface_of(&c.path);
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
        for g in groups {
            let short = shorts.iter().find(|c| interface_of(&c.path) == g);
            let long = longs.iter().find(|c| interface_of(&c.path) == g);
            if poll_link(ctx, pid, short, long, &mut seen) {
                ctx.claim(vid, pid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_match_index_feature_and_software_id() {
        let head = [LONG, 0x01, 0x05, 0x10 | SW_ID];
        let ok = [0x11, 0x01, 0x05, 0x1B, 0x50, 0x00, 0x01];
        assert_eq!(answer(head, &ok), Some(Ok(vec![0x50, 0x00, 0x01])));
        let lighting = [0x11, 0x01, 0x05, 0x17, 0x50, 0x00, 0x01];
        assert_eq!(answer(head, &lighting), None);
        let other_device = [0x11, 0x02, 0x05, 0x1B, 0x50, 0x00, 0x01];
        assert_eq!(answer(head, &other_device), None);
        let err = [0x11, 0x01, 0xFF, 0x05, 0x1B, 0x05, 0x00];
        assert_eq!(answer(head, &err), Some(Err(0x05)));
        let lighting_err = [0x11, 0x01, 0xFF, 0x05, 0x17, 0x05, 0x00];
        assert_eq!(answer(head, &lighting_err), None);
        assert_eq!(answer(head, &[0x11, 0x01]), None);
    }

    #[test]
    fn hidpp10_ping_error_and_registers() {
        let head = [SHORT, 0x02, 0x00, 0x10 | SW_ID];
        assert_eq!(
            answer(head, &[0x10, 0x02, 0x8F, 0x00, 0x1B, 0x01, 0x00]),
            Some(Err(0x01))
        );
        assert_eq!(
            answer(
                [SHORT, 1, 0x81, 0x0D],
                &[0x10, 1, 0x81, 0x0D, 0x40, 0x00, 0x22]
            ),
            Some(Ok(vec![0x40, 0, 0x22]))
        );
        let r = register_percent(&[0x40, 0x00, 0x22]).unwrap();
        assert_eq!((r.percent, r.charging), (Some(64), true));
        assert_eq!(register_percent(&[0x00, 0x00, 0x00]), None);
        let r = register_level(&[7, 0x00]).unwrap();
        assert_eq!((r.percent, r.level, r.charging), (Some(100), "Full", false));
        assert_eq!(register_level(&[3, 0x25]).unwrap().percent, Some(20));
    }

    #[test]
    fn unified_battery_status() {
        let r = unified(&[0x55, 0x04, 0x00]).unwrap();
        assert_eq!((r.percent, r.charging, r.full), (Some(85), false, false));
        let r = unified(&[0x64, 0x08, 0x03]).unwrap();
        assert_eq!((r.percent, r.charging, r.full), (Some(100), true, true));
        let r = unified(&[0x00, 0x02, 0x01]).unwrap();
        assert_eq!((r.percent, r.level, r.charging), (Some(20), "Low", true));
        assert_eq!(unified(&[0x10, 0x00]), None);
    }

    #[test]
    fn battery_status_feature() {
        let r = status(&[0x32, 0x1E, 0x00]).unwrap();
        assert_eq!((r.percent, r.charging), (Some(50), false));
        let r = status(&[0x00, 0x00, 0x03]).unwrap();
        assert_eq!((r.percent, r.full, r.charging), (Some(100), true, true));
        assert_eq!(status(&[0xFF, 0, 0x01]).unwrap().percent, None);
    }

    #[test]
    fn voltage_features() {
        let r = voltage(&[0x0F, 0x9E, 0x80], LI_ION, |b| b & 0x80 != 0).unwrap();
        assert_eq!(
            (r.millivolts, r.percent, r.charging),
            (3998, Some(81), true)
        );
        let r = voltage(&[0x0E, 0xEE, 0x00], headset_curve(0x0A66), |b| b == 3).unwrap();
        assert_eq!((r.millivolts, r.percent), (3822, Some(40)));
        assert_eq!(
            voltage(&[0, 0, 0], LI_ION, |_| false).unwrap().percent,
            None
        );
    }

    #[test]
    fn device_info_identity() {
        let p = [
            0x02, 0x4A, 0x3B, 0x2C, 0x1D, 0x01, 0x01, 0xC5, 0x39, 0x40, 0x7F, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            identity(&p),
            Some(("4A3B2C1D".into(), "C539407F0000".into()))
        );
        assert_eq!(identity(&[0x02, 0, 0, 0, 0, 0x01]), None);
        assert_eq!(
            identity(&[0x02, 1, 2, 3, 4]),
            Some(("01020304".into(), String::new()))
        );
    }

    #[test]
    fn collections_group_by_interface() {
        let a = r"\\?\HID#VID_046D&PID_C547&MI_02&Col01#8&2a2b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let b = r"\\?\HID#VID_046D&PID_C547&MI_02&Col02#8&2a2b3c4d&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let c = r"\\?\HID#VID_046D&PID_C547&MI_02&Col01#8&99999999&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(interface_of(a), interface_of(b));
        assert_ne!(interface_of(a), interface_of(c));
        assert!(is_bluetooth(
            r"\\?\HID#{00001812-0000-1000-8000-00805f9b34fb}_Dev_VID&02046d_PID&b023#9&1&0000#{x}"
        ));
    }

    #[test]
    fn names_and_types() {
        assert_eq!(display_name("G502 LIGHTSPEED"), "Logitech G502 LIGHTSPEED");
        assert_eq!(display_name("MX Master 3"), "MX Master 3");
        assert_eq!(kind_of_type(0x03), Kind::Mouse);
        assert_eq!(kind_of_type(0x00), Kind::Keyboard);
        assert_eq!(kind_of_type(0x08), Kind::Headset);
    }
}
