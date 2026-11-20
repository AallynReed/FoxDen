use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x36B0;
const DONGLE: u16 = 0x3002;

/// PMO keyboards by their own pid, used on the cable and reported by the dongle when paired.
const BOARDS: [(u16, &str); 4] = [
    (0x3009, "WAVE 75"),
    (0x3004, "Aurora 80"),
    (0x302B, "PMO P75"),
    (0x3012, "PMO 80"),
];

fn board_name(pid: u16) -> Option<&'static str> {
    BOARDS.iter().find(|b| b.0 == pid).map(|b| b.1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Firmware {
    /// `AA <cmd> <offset> <len>` frames, battery read inside a START/STOP session.
    Pmo,
    /// VIA custom value 7 on channel 0.
    Via,
}

#[derive(Debug, PartialEq)]
struct Reading {
    level: u8,
    paired: Option<u16>,
}

mod pmo {
    pub const START: u8 = 0x10;
    pub const STOP: u8 = 0x11;
    pub const BATTERY: u8 = 0x30;
    const OK: u8 = 0x55;

    pub fn frame(report_len: usize, cmd: u8, len: u8) -> Vec<u8> {
        let mut f = vec![0u8; report_len];
        f[1..6].copy_from_slice(&[0xAA, cmd, 0x00, 0x00, len]);
        f
    }

    pub fn echo(cmd: u8) -> [u8; 4] {
        [0xAA, cmd, 0x00, 0x00]
    }

    pub fn ok(r: &[u8]) -> bool {
        r.len() > 8 && r[7] == OK
    }

    pub fn level(r: &[u8]) -> Option<u8> {
        (ok(r) && r[..4] == echo(BATTERY) && r[8] <= 100).then(|| r[8])
    }

    /// The dongle's START reply names the paired keyboard's vid and pid.
    pub fn paired(r: &[u8], vid: u16) -> Option<u16> {
        let ids = r.get(9..13)?;
        (ok(r) && u16::from_le_bytes([ids[0], ids[1]]) == vid)
            .then(|| u16::from_le_bytes([ids[2], ids[3]]))
    }
}

/// `Some(None)` when the firmware answers "unhandled" (FF) or no level.
fn via_level(r: &[u8]) -> Option<Option<u8>> {
    match r {
        [0x08, 0x00, 0x07, level, ..] => Some((1..=100).contains(level).then_some(*level)),
        _ => None,
    }
}

/// Writes `req` and returns the first input report that starts with `echo`.
fn exchange(dev: &Dev, req: &[u8], echo: &[u8], ms: u64) -> Option<Vec<u8>> {
    dev.flush();
    if !dev.write(req) {
        return None;
    }
    dev.read_until(Duration::from_millis(ms), |r| {
        (r.len() >= echo.len() + 6 && r.starts_with(echo)).then(|| r.to_vec())
    })
}

impl Firmware {
    fn read(self, dev: &Dev) -> Option<Reading> {
        let report_len = dev.coll.out_len as usize;
        match self {
            Firmware::Pmo => {
                let session = (report_len - 9) as u8;
                let start = pmo::frame(report_len, pmo::START, session);
                let stop = pmo::frame(report_len, pmo::STOP, session);
                let opened = (0..2)
                    .find_map(|_| exchange(dev, &start, &pmo::echo(pmo::START), 600))
                    .filter(|r| pmo::ok(r));
                let Some(opened) = opened else {
                    dev.write(&stop);
                    return None;
                };
                let battery = pmo::frame(report_len, pmo::BATTERY, 0x18);
                let level = exchange(dev, &battery, &pmo::echo(pmo::BATTERY), 600)
                    .as_deref()
                    .and_then(pmo::level);
                exchange(dev, &stop, &pmo::echo(pmo::STOP), 600);
                Some(Reading {
                    level: level?,
                    paired: pmo::paired(&opened, VID),
                })
            }
            Firmware::Via => {
                let mut req = vec![0u8; report_len];
                req[1..4].copy_from_slice(&[0x08, 0x00, 0x07]);
                let r = (0..2).find_map(|_| exchange(dev, &req, &[0x08, 0x00, 0x07], 600))?;
                Some(Reading {
                    level: via_level(&r).flatten()?,
                    paired: None,
                })
            }
        }
    }
}

/// Per product: which firmware answered, and how long to leave a quiet one alone.
#[derive(Default)]
struct Memory {
    firmware: Option<Firmware>,
    quiet: u32,
    rest_until: Option<Instant>,
    paired: Option<u16>,
}

/// Quiet polls before a known firmware is forgotten (another keyboard may be paired by then).
const FORGET_AFTER: u32 = 5;

fn retry_after(quiet: u32, known: bool) -> Duration {
    if known {
        Duration::from_secs(15)
    } else {
        Duration::from_secs((20u64 << quiet.min(4)).min(240))
    }
}

impl Memory {
    fn resting(&self) -> bool {
        self.rest_until.is_some_and(|t| Instant::now() < t)
    }

    fn heard(&mut self, fw: Firmware, reading: &Reading) {
        *self = Memory {
            firmware: Some(fw),
            paired: reading.paired.or(self.paired),
            ..Memory::default()
        };
    }

    fn silent(&mut self) {
        self.quiet += 1;
        if self.quiet >= FORGET_AFTER {
            self.firmware = None;
        }
        self.rest_until =
            Some(Instant::now() + retry_after(self.quiet - 1, self.firmware.is_some()));
    }
}

static MEMORY: Mutex<BTreeMap<u16, Memory>> = Mutex::new(BTreeMap::new());

fn raw_hid(c: &Coll) -> bool {
    c.iface == 1
        && c.page == 0xFF60
        && c.usage == 0x0061
        && c.in_len == c.out_len
        && matches!(c.out_len, 33 | 65)
}

fn via_name(product: &str) -> String {
    let trimmed = [" 2.4G", " Wireless"].iter().fold(product, |n, s| {
        n.strip_suffix(s).filter(|t| !t.is_empty()).unwrap_or(n)
    });
    if trimmed.is_empty() || product.contains("Dongle") {
        "Wireless keyboard".into()
    } else {
        trimmed.into()
    }
}

/// Asks one product; returns true when it answered.
fn poll_port(
    ctx: &mut Ctx,
    memory: &mut BTreeMap<u16, Memory>,
    pid: u16,
    cable_answered: bool,
) -> bool {
    let cable = pid != DONGLE;
    if ctx.is_claimed(VID, pid) {
        return false;
    }
    let colls = ctx.find(VID, pid, |c| raw_hid(c) && (cable || c.out_len == 33));
    if colls.is_empty() {
        memory.remove(&pid);
        return false;
    }
    let mem = memory.entry(pid).or_default();
    // A PMO keyboard on its cable leaves its dongle silent.
    if !cable && cable_answered && mem.firmware != Some(Firmware::Via) {
        ctx.claim(VID, pid);
        return false;
    }
    if mem.resting() {
        return false;
    }
    let candidates: Vec<Firmware> = match mem.firmware {
        _ if cable => vec![Firmware::Pmo],
        Some(fw) => vec![fw],
        None if colls[0].product.contains("URX") => vec![Firmware::Via, Firmware::Pmo],
        None => vec![Firmware::Pmo, Firmware::Via],
    };
    // The dongle only has one raw collection worth asking; a cable may have several.
    let tried = if cable { &colls[..] } else { &colls[..1] };
    for c in tried {
        let Some(dev) = Dev::open(c) else { continue };
        let Some((fw, reading)) = candidates
            .iter()
            .find_map(|fw| fw.read(&dev).map(|r| (*fw, r)))
        else {
            continue;
        };
        mem.heard(fw, &reading);
        let key_pid = match fw {
            Firmware::Pmo if !cable => mem.paired.unwrap_or(DONGLE),
            _ => pid,
        };
        let name = match fw {
            Firmware::Via => via_name(&c.product),
            Firmware::Pmo => board_name(key_pid).unwrap_or("Wireless keyboard").into(),
        };
        let b = Battery {
            key: ctx.stable_key(&format!("via-{}:{}", hex4(VID), hex4(key_pid))),
            name,
            via: if cable {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Keyboard,
            percent: Some(reading.level),
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, VID, pid);
        return true;
    }
    mem.silent();
    false
}

pub fn poll_via_dongle(ctx: &mut Ctx) {
    let mut memory = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let mut cable_answered = false;
    for (pid, _) in BOARDS {
        cable_answered |= poll_port(ctx, &mut memory, pid, false);
    }
    poll_port(ctx, &mut memory, DONGLE, cable_answered);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmo_session_frames() {
        let f = pmo::frame(33, pmo::START, 0x18);
        assert_eq!(&f[..6], &[0x00, 0xAA, 0x10, 0x00, 0x00, 0x18]);
        assert_eq!(f.len(), 33);
    }

    #[test]
    fn pmo_battery_and_pairing() {
        let mut bat = [0u8; 32];
        bat[..9].copy_from_slice(&[0xAA, 0x30, 0, 0, 0x18, 0, 0, 0x55, 78]);
        assert_eq!(pmo::level(&bat), Some(78));
        bat[7] = 0x0F;
        assert_eq!(pmo::level(&bat), None);

        let mut start = [0u8; 32];
        start[..13].copy_from_slice(&[
            0xAA, 0x10, 0, 0, 0x18, 0, 0, 0x55, 0x00, 0xB0, 0x36, 0x09, 0x30,
        ]);
        assert_eq!(pmo::paired(&start, VID), Some(0x3009));
        assert_eq!(pmo::paired(&start, 0x1234), None);
    }

    #[test]
    fn via_custom_value() {
        assert_eq!(via_level(&[0x08, 0x00, 0x07, 64, 0]), Some(Some(64)));
        assert_eq!(via_level(&[0x08, 0x00, 0x07, 0xFF, 0]), Some(None));
        assert_eq!(via_level(&[0x07, 0x00, 0x07, 64]), None);
    }

    #[test]
    fn via_names() {
        assert_eq!(via_name("EVO80 2.4G"), "EVO80");
        assert_eq!(via_name("URX Core75 Wireless"), "URX Core75");
        assert_eq!(via_name("Wireless 2.4G Dongle"), "Wireless keyboard");
    }

    #[test]
    fn quiet_products_rest_longer() {
        assert_eq!(retry_after(0, false), Duration::from_secs(20));
        assert_eq!(retry_after(2, false), Duration::from_secs(80));
        assert_eq!(retry_after(9, false), Duration::from_secs(240));
        assert_eq!(retry_after(9, true), Duration::from_secs(15));
        let mut m = Memory {
            firmware: Some(Firmware::Via),
            ..Memory::default()
        };
        for _ in 0..FORGET_AFTER {
            m.silent();
        }
        assert!(m.firmware.is_none() && m.resting());
    }
}
