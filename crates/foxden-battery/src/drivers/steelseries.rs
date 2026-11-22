use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::{clamp100, map_to_100};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1038;

/// What one query found; `connected: false` when the receiver answered but the device is off.
#[derive(Debug, Default, PartialEq)]
struct Status {
    name: &'static str,
    percent: Option<u8>,
    charging: bool,
    connected: bool,
}

fn on(name: &'static str, percent: Option<u8>, charging: bool) -> Status {
    Status {
        name,
        percent,
        charging,
        connected: true,
    }
}

fn off(name: &'static str) -> Status {
    Status {
        name,
        ..Status::default()
    }
}

fn arctis_1(r: &[u8]) -> Option<Status> {
    let name = "SteelSeries Arctis 1 Wireless";
    let [_, _, link, pct, ..] = *r else {
        return None;
    };
    Some(if link == 0x01 {
        off(name)
    } else {
        on(name, Some(clamp100(pct as i32)), false)
    })
}

fn arctis_7(pid: u16, r: &[u8]) -> Option<Status> {
    let name = if pid == 0x1252 {
        "SteelSeries Arctis Pro"
    } else {
        "SteelSeries Arctis 7"
    };
    let [_, _, pct, ..] = *r else { return None };
    Some(if pct == 0 {
        off(name)
    } else {
        on(name, Some(clamp100(pct as i32)), false)
    })
}

fn arctis_9(r: &[u8]) -> Option<Status> {
    let [_, _, _, raw, ch, ..] = *r else {
        return None;
    };
    Some(on(
        "SteelSeries Arctis 9",
        Some(map_to_100(raw as i32, 0x64, 0x9A)),
        ch == 0x01,
    ))
}

fn nova_pro(r: &[u8]) -> Option<Status> {
    let name = "SteelSeries Arctis Nova Pro Wireless";
    if r.len() < 16 {
        return None;
    }
    Some(match r[15] {
        0x01 => off(name),
        st => on(name, Some(map_to_100(r[6] as i32, 0, 8)), st == 0x02),
    })
}

const NOVA_7_DISCRETE: &[u16] = &[0x2202, 0x2206, 0x220A, 0x223A, 0x227A, 0x22A4];

/// The `00 B0` status block of the Nova family; its layout depends on the model.
fn nova(pid: u16, r: &[u8]) -> Option<Status> {
    if r.len() < 7 || r[0] != 0xB0 {
        return None;
    }
    let s = match pid {
        0x220E | 0x2212 | 0x2216 | 0x2236 => {
            let name = "SteelSeries Arctis 7+";
            if r[1] == 0x01 {
                off(name)
            } else {
                on(name, Some(map_to_100(r[2] as i32, 0, 4)), r[3] == 0x01)
            }
        }
        0x2269 | 0x226D | 0x2232 | 0x2253 => {
            let name = if matches!(pid, 0x2232 | 0x2253) {
                "SteelSeries Arctis Nova 5"
            } else {
                "SteelSeries Arctis Nova 3P Wireless"
            };
            if r[1] == 0x02 {
                off(name)
            } else {
                on(name, Some(clamp100(r[3] as i32)), r[4] == 0x01)
            }
        }
        0x230A => {
            let name = "SteelSeries Arctis GameBuds";
            let left = (r[3] == 0x03).then_some(r[5]);
            let right = (r[4] == 0x03).then_some(r[6]);
            match left.into_iter().chain(right).min() {
                Some(l) => on(name, Some(clamp100(l as i32)), false),
                None => off(name),
            }
        }
        _ => {
            let name = "SteelSeries Arctis Nova 7";
            if r[3] == 0x00 {
                off(name)
            } else {
                let pct = if NOVA_7_DISCRETE.contains(&pid) {
                    map_to_100(r[2] as i32, 0, 4)
                } else {
                    clamp100(r[2] as i32)
                };
                on(name, Some(pct), matches!(r[3], 0x01 | 0x02))
            }
        }
    };
    Some(s)
}

struct Aerox {
    pid: u16,
    /// The receiver's pid: one identity on the cable and the receiver.
    home: u16,
    name: &'static str,
    wired: bool,
    /// Percent layout instead of 5% steps.
    percent: bool,
}

const fn aerox(pid: u16, home: u16, name: &'static str, wired: bool, percent: bool) -> Aerox {
    Aerox {
        pid,
        home,
        name,
        wired,
        percent,
    }
}

const AEROX: &[Aerox] = &[
    aerox(0x1838, 0x1838, "SteelSeries Aerox 3 Wireless", false, false),
    aerox(0x183A, 0x1838, "SteelSeries Aerox 3 Wireless", true, false),
    aerox(
        0x1878,
        0x1878,
        "SteelSeries Aerox 3 Wireless CS2 Dragon Lore Edition",
        false,
        false,
    ),
    aerox(
        0x187A,
        0x1878,
        "SteelSeries Aerox 3 Wireless CS2 Dragon Lore Edition",
        true,
        false,
    ),
    aerox(
        0x1890,
        0x1890,
        "SteelSeries Aerox 3 Wireless Gen 2",
        false,
        true,
    ),
    aerox(
        0x1892,
        0x1890,
        "SteelSeries Aerox 3 Wireless Gen 2",
        true,
        true,
    ),
    aerox(0x1852, 0x1852, "SteelSeries Aerox 5 Wireless", false, false),
    aerox(0x1854, 0x1852, "SteelSeries Aerox 5 Wireless", true, false),
    aerox(0x1858, 0x1858, "SteelSeries Aerox 9 Wireless", false, false),
    aerox(0x185A, 0x1858, "SteelSeries Aerox 9 Wireless", true, false),
    aerox(0x1840, 0x1840, "SteelSeries Prime Wireless", false, false),
    aerox(0x1842, 0x1840, "SteelSeries Prime Wireless", true, false),
    aerox(
        0x1848,
        0x1848,
        "SteelSeries Prime Mini Wireless",
        false,
        false,
    ),
    aerox(
        0x184A,
        0x1848,
        "SteelSeries Prime Mini Wireless",
        true,
        false,
    ),
    aerox(
        0x1860,
        0x1860,
        "SteelSeries Aerox 5 Wireless Diablo IV Edition",
        false,
        false,
    ),
];

/// Aerox reply after the command echo. `None` when the mouse is off or asleep, or the bytes make no sense.
fn aerox_status(m: &Aerox, r: &[u8]) -> Option<(Option<u8>, bool)> {
    if m.percent {
        let [_, state @ 0..=2, pct, ..] = *r else {
            return None;
        };
        if !m.wired && state == 0 && pct == 0 {
            return None;
        }
        return Some(((pct <= 100).then_some(pct), state != 0));
    }
    let [_, b, ..] = *r else { return None };
    let step = b & 0x7F;
    match step {
        0 => None,
        1..=21 => Some((Some((step - 1) * 5), b & 0x80 != 0)),
        _ => Some((None, false)),
    }
}

fn rival(pid: u16, r: &[u8]) -> Option<Status> {
    let [pct, _, ch, ..] = *r else { return None };
    let name = match pid {
        0x172B | 0x1726 => "SteelSeries Rival 650 Wireless",
        0x1872 => "SteelSeries Rival 3 Wireless Gen 2",
        _ => "SteelSeries Rival 3 Wireless",
    };
    Some(on(name, Some(clamp100(pct as i32)), ch != 0))
}

fn rival_pro(r: &[u8]) -> Option<u8> {
    match *r {
        [0x92, 0x00, pct @ 0..=100, ..] => Some(pct),
        _ => None,
    }
}

fn write_read(dev: &Dev, req: &[u8], wait: Duration) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(req) {
        return None;
    }
    dev.read(wait).filter(|r| !r.is_empty())
}

/// Resends `cmd` every 100 ms until its echo comes back: the vendor app and lighting share this interface.
/// `40 FF` is the receiver saying it cannot reach the mouse.
fn query_echo(dev: &Dev, cmd: u8, min_len: usize) -> Option<Vec<u8>> {
    dev.flush();
    let end = Instant::now() + Duration::from_secs(1);
    let mut resend = Instant::now();
    loop {
        let now = Instant::now();
        if now >= end {
            return None;
        }
        if now >= resend {
            if !dev.write(&[0x00, cmd]) {
                return None;
            }
            resend = now + Duration::from_millis(100);
        }
        let Some(r) = dev.read(resend.min(end).saturating_duration_since(Instant::now())) else {
            continue;
        };
        if r.len() >= min_len && r[0] == cmd {
            return Some(r);
        }
        if r.starts_with(&[0x40, 0xFF]) {
            return None;
        }
    }
}

fn headset(pid: u16, s: Status) -> Battery {
    Battery {
        key: format!("steelseries-1038:{pid:04X}"),
        name: s.name.into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Headset,
        percent: s.percent.filter(|_| s.connected),
        charging: s.charging,
        connected: s.connected,
        ..Battery::default()
    }
}

fn mouse(key_pid: u16, name: &str, wired: bool) -> Battery {
    Battery {
        key: format!("steelseries-1038:{key_pid:04X}"),
        name: name.into(),
        via: if wired {
            "USB cable"
        } else {
            "2.4 GHz receiver"
        }
        .into(),
        kind: Kind::Mouse,
        ..Battery::default()
    }
}

/// Arctis Pro Wireless's interface 0 does not number its reports; a numbered firmware is tried second.
fn arctis_pro_cmd(dev: &Dev, cmd: u8) -> Option<Vec<u8>> {
    let wait = Duration::from_secs(1);
    write_read(dev, &[0x00, cmd, 0xAA], wait).or_else(|| write_read(dev, &[cmd, 0xAA], wait))
}

fn query(dev: &Dev, pid: u16) -> Option<Battery> {
    let ms = Duration::from_millis;
    let hs = |s: Option<Status>| s.map(|s| headset(pid, s));
    match pid {
        0x12B3 | 0x12B6 | 0x12D7 => hs(arctis_1(&write_read(dev, &[0x06, 0x12], ms(1000))?)),
        0x1260 | 0x12AD | 0x1252 => hs(arctis_7(pid, &write_read(dev, &[0x06, 0x18], ms(1000))?)),
        0x12C2 => hs(arctis_9(&write_read(dev, &[0x00, 0x20], ms(1000))?)),
        0x1290 => {
            let name = "SteelSeries Arctis Pro Wireless";
            let link = arctis_pro_cmd(dev, 0x41)?;
            if link[0] == 0x02 {
                return Some(headset(pid, off(name)));
            }
            let pct = arctis_pro_cmd(dev, 0x40)
                .filter(|r| r[0] <= 4)
                .map(|r| map_to_100(r[0] as i32, 0, 4));
            Some(headset(pid, on(name, pct, false)))
        }
        0x12E0 | 0x225D => hs(nova_pro(&write_read(dev, &[0x06, 0xB0], ms(1000))?)),
        0x1830 | 0x1872 | 0x172B | 0x1726 => {
            let s = rival(pid, &write_read(dev, &[0x00, 0xAA, 0x01], ms(300))?)?;
            let key_pid = if matches!(pid, 0x172B | 0x1726) {
                0x172B
            } else {
                pid
            };
            let mut b = mouse(key_pid, s.name, pid == 0x172B);
            b.percent = s.percent;
            b.charging = s.charging;
            Some(b)
        }
        0x1880 => {
            dev.flush();
            if !dev.write(&[0x00, 0x92]) {
                return None;
            }
            let end = Instant::now() + ms(500);
            let pct = loop {
                let left = end.checked_duration_since(Instant::now())?;
                let r = dev.read(left)?;
                if r.len() >= 3 && r[..2] == [0x92, 0x00] {
                    break rival_pro(&r)?;
                }
            };
            let mut b = mouse(pid, "SteelSeries Rival Pro", false);
            b.via = "UWB base station".into();
            b.percent = Some(pct);
            Some(b)
        }
        _ if NOVA_PIDS.contains(&pid) => hs(nova(pid, &write_read(dev, &[0x00, 0xB0], ms(1000))?)),
        _ => {
            let m = AEROX.iter().find(|m| m.pid == pid)?;
            let r = query_echo(
                dev,
                if m.wired { 0x92 } else { 0xD2 },
                if m.percent { 3 } else { 2 },
            )?;
            let (percent, charging) = aerox_status(m, &r)?;
            let mut b = mouse(m.home, m.name, m.wired);
            b.grace_secs = 1800;
            b.percent = percent;
            b.charging = charging;
            Some(b)
        }
    }
}

const NOVA_PIDS: &[u16] = &[
    0x220E, 0x2212, 0x2216, 0x2236, 0x2269, 0x226D, 0x2232, 0x2253, 0x230A, 0x2202, 0x22A1, 0x227E,
    0x2206, 0x2258, 0x229E, 0x22AD, 0x223A, 0x22A9, 0x227A, 0x22A4, 0x22A5, 0x220A, 0x22A7,
];

/// Which collection carries a model's battery query: a vendor usage, or an interface with an OUT report.
#[derive(Clone, Copy)]
enum Pick {
    Usage(u16, u16),
    Iface(i32),
}

impl Pick {
    fn accepts(self, c: &Coll) -> bool {
        match self {
            Pick::Usage(page, usage) => c.page == page && c.usage == usage,
            Pick::Iface(n) => c.iface == n && c.out_len > 0,
        }
    }
}

fn collection(pid: u16) -> Option<Pick> {
    Some(match pid {
        0x12B3 | 0x12B6 | 0x12D7 | 0x1260 | 0x12AD | 0x1252 => Pick::Usage(0xFF43, 0x0202),
        0x12C2 | 0x1290 | 0x172B | 0x1726 => Pick::Iface(0),
        0x12E0 | 0x225D => Pick::Iface(4),
        0x1830 | 0x1872 => Pick::Iface(3),
        0x1880 => Pick::Iface(2),
        _ if NOVA_PIDS.contains(&pid) => Pick::Usage(0xFFC0, 0x0001),
        _ if AEROX.iter().any(|m| m.pid == pid) => Pick::Iface(3),
        _ => return None,
    })
}

pub fn poll_steel_series(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        let Some(pick) = collection(pid) else {
            continue;
        };
        for c in ctx.find(vid, pid, |c| pick.accepts(c)) {
            let Some(b) = Dev::open(&c).and_then(|d| {
                let _turn = foxden_core::hid_turn::take();
                query(&d, pid)
            }) else {
                continue;
            };
            if let Some(o) = ctx.out.iter_mut().find(|o| o.key == b.key) {
                if b.via == "USB cable" {
                    *o = Battery { vid, ..b };
                }
                ctx.claim(vid, pid);
            } else {
                let key = ctx.stable_key(&b.key);
                ctx.add(Battery { key, ..b }, vid, pid);
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arctis_headsets() {
        assert_eq!(
            arctis_1(&[0x06, 0x12, 0x00, 0x46]),
            Some(on("SteelSeries Arctis 1 Wireless", Some(70), false))
        );
        assert!(!arctis_1(&[0x06, 0x12, 0x01, 0x00]).unwrap().connected);
        assert_eq!(
            arctis_7(0x1252, &[0x06, 0x18, 0x32]).unwrap().name,
            "SteelSeries Arctis Pro"
        );
        assert_eq!(
            arctis_9(&[0x00, 0x20, 0x00, 0x7F, 0x01]).unwrap(),
            on("SteelSeries Arctis 9", Some(50), true)
        );
        let mut r = [0u8; 16];
        r[6] = 4;
        r[15] = 0x02;
        assert_eq!(
            nova_pro(&r),
            Some(on("SteelSeries Arctis Nova Pro Wireless", Some(50), true))
        );
        r[15] = 0x01;
        assert!(!nova_pro(&r).unwrap().connected);
    }

    #[test]
    fn nova_layouts() {
        let s = nova(0x2202, &[0xB0, 0x00, 0x03, 0x01, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!((s.percent, s.charging), (Some(75), true));
        let s = nova(0x22A1, &[0xB0, 0x00, 0x58, 0x03, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(
            (s.percent, s.charging, s.connected),
            (Some(88), false, true)
        );
        assert!(
            !nova(0x22A1, &[0xB0, 0, 0x58, 0x00, 0, 0, 0])
                .unwrap()
                .connected
        );
        let s = nova(0x2232, &[0xB0, 0x00, 0x00, 0x3C, 0x01, 0x00, 0x00]).unwrap();
        assert_eq!(
            (s.name, s.percent, s.charging),
            ("SteelSeries Arctis Nova 5", Some(60), true)
        );
        let s = nova(0x220E, &[0xB0, 0x00, 0x02, 0x00, 0, 0, 0]).unwrap();
        assert_eq!(s.percent, Some(50));
        let s = nova(0x230A, &[0xB0, 0, 0, 0x03, 0x03, 0x50, 0x40]).unwrap();
        assert_eq!(s.percent, Some(64));
        assert!(
            !nova(0x230A, &[0xB0, 0, 0, 0x01, 0x01, 0x50, 0x40])
                .unwrap()
                .connected
        );
    }

    #[test]
    fn aerox_layouts() {
        let steps = &AEROX[0];
        assert_eq!(aerox_status(steps, &[0xD2, 0x0A]), Some((Some(45), false)));
        assert_eq!(aerox_status(steps, &[0x92, 0x8B]), Some((Some(50), true)));
        assert_eq!(aerox_status(steps, &[0xD2, 0x00]), None);
        let gen2 = &AEROX[4];
        assert_eq!(
            aerox_status(gen2, &[0xD2, 0x01, 0x48]),
            Some((Some(72), true))
        );
        assert_eq!(aerox_status(gen2, &[0xD2, 0x00, 0xFF]), Some((None, false)));
        assert_eq!(aerox_status(gen2, &[0xD2, 0x00, 0x00]), None);
        assert_eq!(aerox_status(gen2, &[0xD2, 0x05, 0x10]), None);
        let mini = AEROX.iter().find(|m| m.pid == 0x184A).unwrap();
        assert_eq!((mini.home, mini.wired), (0x1848, true));
        assert_eq!(aerox_status(mini, &[0x92, 0x95]), Some((Some(100), true)));
        assert_eq!(collection(0x1848).map(|_| ()), Some(()));
    }

    #[test]
    fn rival_mice() {
        assert_eq!(
            rival(0x1830, &[0x55, 0x00, 0x01]),
            Some(on("SteelSeries Rival 3 Wireless", Some(85), true))
        );
        assert_eq!(rival_pro(&[0x92, 0x00, 0x3C, 0x00, 0x50]), Some(60));
        assert_eq!(rival_pro(&[0x92, 0x00, 0xFF]), None);
        assert!(collection(0x1838).is_some() && collection(0x1234).is_none());
    }
}
