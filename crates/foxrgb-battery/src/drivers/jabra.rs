use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x0B0E;
/// Headset behind a Link dongle, and a headset on its own cable.
const VIA_DONGLE: u8 = 0x04;
const VIA_USB: u8 = 0x08;
const NAK_PEER_OFFLINE: u8 = 0xF3;
const SILENT_RETRY: Duration = Duration::from_secs(5 * 60);

const DECT_BASES: &[u16] = &[
    0x1000, 0x1016, 0x1017, 0x1018, 0x1021, 0x1022, 0x1023, 0x1037, 0x1038, 0x1039, 0x1041, 0x1042,
    0x1043, 0x1110, 0x1112, 0x1113, 0x1118, 0x111A, 0x111B, 0x111D, 0x111E, 0x111F, 0x1120, 0x1121,
    0x1122, 0x1123, 0x1124, 0x1126, 0x1131, 0x1132, 0x1133, 0x1134, 0x1135, 0x1136, 0x1140, 0x1141,
    0x1142, 0x1146, 0x1147, 0x1148, 0x114A, 0x114B, 0x114C, 0x114D, 0x114E, 0x114F, 0x1150, 0x1151,
    0xA700, 0xA701, 0xA702, 0xA703, 0xA704, 0xA705, 0xA706, 0xAE60, 0xAE62, 0xAE63,
];

#[derive(Debug, PartialEq)]
enum Reply {
    Ok(Vec<u8>),
    Nak(u8),
}

/// GNP reply `05 00 [src] [seq] [C0|len] [class] [op] [payload]`; class FE is a NAK with the reason in `[6]`.
fn parse_reply(r: &[u8], dst: u8, seq: u8, class: u8, op: u8) -> Option<Reply> {
    if r.len() < 7
        || r[0] != 0x05
        || r[1] != 0x00
        || r[2] != dst
        || r[3] != seq
        || r[4] & 0xC0 != 0xC0
    {
        return None;
    }
    if r[5] == 0xFE {
        return Some(Reply::Nak(r[6]));
    }
    let len = (r[4] & 0x3F) as usize;
    if r[5] != class || r[6] != op || len < 6 || len + 1 > r.len() {
        return None;
    }
    Some(Reply::Ok(r[7..len + 1].to_vec()))
}

/// Battery payload `[flags][level]`: flags bit 0 = charging.
fn parse_battery(p: &[u8]) -> Option<(Option<u8>, bool)> {
    (p.len() >= 2).then(|| ((p[1] <= 100).then_some(p[1]), p[0] & 0x01 != 0))
}

/// Name payload `[n][n bytes UTF-8]`.
fn parse_name(p: &[u8]) -> Option<String> {
    let n = *p.first()? as usize;
    (n > 0 && p.len() > n).then(|| String::from_utf8_lossy(&p[1..=n]).into_owned())
}

static SEQ: AtomicU8 = AtomicU8::new(0x20);

fn query(d: &Dev, dst: u8, class: u8, op: u8, ms: u64) -> Option<Reply> {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    d.flush();
    if !d.write_timeout(
        &[0x05, dst, 0x00, seq, 0x46, class, op],
        Duration::from_millis(300),
    ) {
        return None;
    }
    d.read_until(Duration::from_millis(ms), |r| {
        parse_reply(r, dst, seq, class, op)
    })
}

#[derive(Default)]
struct Memory {
    address: BTreeMap<u16, u8>,
    skip: BTreeMap<u16, Option<Instant>>,
    name: BTreeMap<u16, String>,
}

static MEMORY: Mutex<Option<Memory>> = Mutex::new(None);

pub fn poll_jabra(ctx: &mut Ctx) {
    let mut guard = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let mem = guard.get_or_insert_with(Memory::default);
    let now = Instant::now();
    for (_, pid) in ctx.products(VID) {
        if DECT_BASES.contains(&pid) {
            continue;
        }
        if let Some(until) = mem.skip.get(&pid)
            && until.is_none_or(|t| now < t)
        {
            continue;
        }
        let mut colls = ctx.find(VID, pid, |c| {
            c.page == 0xFF00 && matches!(c.usage, 0x05 | 0x01) && c.out_len >= 64 && c.in_len >= 64
        });
        colls.sort_by_key(|c| c.usage != 0x05);
        let (mut reported, mut probed, mut skippable, mut all_nak) = (false, false, true, true);
        for c in colls {
            let Some(d) = Dev::open(&c) else { continue };
            let known = mem.address.get(&pid).copied();
            let dongle = known.map_or_else(|| c.product.contains("Link"), |a| a == VIA_DONGLE);
            let order: &[u8] = match (known, dongle) {
                (Some(VIA_DONGLE), _) => &[VIA_DONGLE],
                (Some(_), _) => &[VIA_USB],
                (None, true) => &[VIA_DONGLE, VIA_USB],
                (None, false) => &[VIA_USB, VIA_DONGLE],
            };
            if dongle || known.is_some() {
                skippable = false;
            }
            let mut found = None;
            for &a in order {
                match query(&d, a, 0x12, 0x02, 700) {
                    Some(Reply::Ok(p)) if p.len() >= 2 => {
                        found = Some((a, parse_battery(&p)));
                        break;
                    }
                    Some(Reply::Nak(why))
                        if a == VIA_DONGLE && (dongle || why == NAK_PEER_OFFLINE) =>
                    {
                        found = Some((a, None));
                        break;
                    }
                    Some(Reply::Nak(_)) => {}
                    _ => all_nak = false,
                }
            }
            let Some((address, battery)) = found else {
                mem.address.remove(&pid);
                probed = true;
                continue;
            };
            mem.address.insert(pid, address);
            let mut b = Battery {
                key: ctx.stable_key(&format!("jabra-{}:{}", hex4(VID), hex4(pid))),
                via: if address == VIA_DONGLE {
                    "Jabra Link dongle"
                } else {
                    "USB cable"
                }
                .into(),
                kind: Kind::Headset,
                ..Battery::default()
            };
            match battery {
                Some((percent, charging)) => {
                    b.percent = percent;
                    b.charging = charging;
                    let name = mem.name.entry(pid).or_insert_with(|| {
                        let asked = match query(&d, address, 0x02, 0x00, 400) {
                            Some(Reply::Ok(p)) => parse_name(&p),
                            _ => None,
                        };
                        asked.unwrap_or_else(|| "Jabra headset".into())
                    });
                    b.name = name.clone();
                }
                None => {
                    b.connected = false;
                    b.name = mem
                        .name
                        .remove(&pid)
                        .unwrap_or_else(|| "Jabra headset".into());
                }
            }
            ctx.add(b, VID, pid);
            reported = true;
            break;
        }
        if !reported && probed && skippable {
            mem.skip.insert(
                pid,
                if all_nak {
                    None
                } else {
                    Some(now + SILENT_RETRY)
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_reply_and_nak() {
        let r = [0x05, 0x00, 0x04, 0x21, 0xC8, 0x12, 0x02, 0x01, 0x4B, 0, 0];
        let Some(Reply::Ok(p)) = parse_reply(&r, 0x04, 0x21, 0x12, 0x02) else {
            panic!()
        };
        assert_eq!(parse_battery(&p), Some((Some(75), true)));
        assert_eq!(parse_reply(&r, 0x04, 0x22, 0x12, 0x02), None);
        let nak = [0x05, 0x00, 0x04, 0x21, 0xC7, 0xFE, 0xF3, 0];
        assert_eq!(
            parse_reply(&nak, 0x04, 0x21, 0x12, 0x02),
            Some(Reply::Nak(0xF3))
        );
    }

    #[test]
    fn headset_name_payload() {
        assert_eq!(
            parse_name(&[5, b'E', b'v', b'o', b'l', b'v', 0]),
            Some("Evolv".into())
        );
        assert_eq!(parse_name(&[9, b'E']), None);
        assert_eq!(parse_name(&[0]), None);
    }
}
