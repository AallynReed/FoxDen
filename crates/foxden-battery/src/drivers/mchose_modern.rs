use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3837;
const KEYBOARD_RX: u16 = 0x1015;
/// (pid, via, keyboard)
const TARGETS: [(u16, &str, bool); 7] = [
    (0x1014, "2.4 GHz receiver", false),
    (0x1018, "2.4 GHz receiver", false),
    (0x1016, "2.4 GHz receiver", false),
    (0x102C, "2.4 GHz receiver", false),
    (0x102D, "2.4 GHz receiver", false),
    (0x4028, "USB cable", false),
    (KEYBOARD_RX, "2.4 GHz receiver", true),
];

const REPORT: u8 = 0x4D;
const DEVICE_INFO: u16 = 0x0900;
/// First data byte in both request and reply.
const DATA: usize = 8;
/// Idle keyboards are asked this rarely so polling doesn't keep them awake.
const KEYBOARD_EVERY: Duration = Duration::from_secs(180);

fn xor(b: &[u8]) -> u8 {
    b.iter().fold(0, |a, x| a ^ x)
}

fn request(len: usize) -> Vec<u8> {
    let mut req = vec![0u8; len.max(DATA + 1)];
    let [lo, hi] = DEVICE_INFO.to_le_bytes();
    req[..6].copy_from_slice(&[REPORT, 0x01, 0x01, 0x00, lo, hi]);
    req[DATA] = xor(&req[2..DATA]);
    req
}

#[derive(Debug, PartialEq)]
enum Info {
    Unlinked,
    Linked { percent: u8, charging: bool },
}

/// A device-info frame; `None` for anything else, including bad checksums.
fn parse(r: &[u8], keyboard: bool) -> Option<Info> {
    if r.len() < DATA + 2 || r[0] != REPORT || u16::from_le_bytes([r[4], r[5]]) != DEVICE_INFO {
        return None;
    }
    let len = r[3] as usize;
    if DATA + len >= r.len() {
        return None;
    }
    // With the device off the receiver sends a header, one 0xFF and zeros.
    if len < 14 {
        return (!keyboard || (len == 1 && r[DATA] == 0xFF)).then_some(Info::Unlinked);
    }
    let d = &r[DATA..DATA + len];
    if r[DATA + len] != xor(&r[2..DATA + len]) || u16::from_le_bytes([d[0], d[1]]) != VID {
        return None;
    }
    if keyboard && (len < 20 || d[2..4] != [0x23, 0x20]) {
        return None;
    }
    if d[10] == 0 {
        return Some(Info::Unlinked);
    }
    (d[12] <= 100).then_some(Info::Linked {
        percent: d[12],
        charging: d[11] == 1,
    })
}

fn query(dev: &Dev, keyboard: bool) -> Option<Info> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let req = request(dev.coll.out_len as usize);
    for attempt in 0..3 {
        if attempt > 0 {
            if deadline.saturating_duration_since(Instant::now()) <= Duration::from_millis(150) {
                break;
            }
            sleep(Duration::from_millis(150));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let window = left.min(Duration::from_millis(250));
        if !dev.write_timeout(&req, window) {
            continue;
        }
        let left = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        if let Some(info) = dev.read_until(left, |r| parse(r, keyboard)) {
            return Some(info);
        }
    }
    None
}

/// Last keyboard reading by collection path, and when to ask again.
static KEYBOARDS: Mutex<BTreeMap<String, (Instant, Battery)>> = Mutex::new(BTreeMap::new());

fn matches(c: &Coll, keyboard: bool) -> bool {
    if keyboard {
        c.page == 0xFF80 && c.usage == 1 && c.out_len == 64 && c.in_len == 64
    } else {
        (c.page == 0xFF01 || c.page == 0xFF0B) && c.out_len >= 64 && c.in_len >= 21
    }
}

fn display_name(product: &str, keyboard: bool) -> String {
    if product.is_empty() {
        return if keyboard {
            "MCHOSE K87 V3"
        } else {
            "MCHOSE mouse"
        }
        .into();
    }
    if keyboard && let Some(n) = product.strip_suffix(" 2.4G").filter(|n| !n.is_empty()) {
        return n.into();
    }
    product.into()
}

pub fn poll_mchose_modern(ctx: &mut Ctx) {
    let mut cache = KEYBOARDS.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|path, _| {
        ctx.colls
            .iter()
            .any(|c| c.vid == VID && c.pid == KEYBOARD_RX && &c.path == path)
    });

    for (pid, via, keyboard) in TARGETS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        for c in ctx.find(VID, pid, |c| matches(c, keyboard)) {
            let key = ctx.stable_key(&format!("mchose-{}:{}", hex4(VID), hex4(pid)));
            if let Some((next, last)) = cache.get(&c.path)
                && Instant::now() < *next
            {
                let b = Battery {
                    key,
                    ..last.clone()
                };
                ctx.add(b, VID, pid);
                break;
            }
            let Some(dev) = Dev::open(&c) else { continue };
            dev.flush();
            let info = query(&dev, keyboard);
            if info.is_none() && !keyboard {
                continue;
            }
            let mut b = Battery {
                key,
                name: display_name(&c.product, keyboard),
                via: via.into(),
                kind: if keyboard {
                    Kind::Keyboard
                } else {
                    Kind::Mouse
                },
                grace_secs: 1800,
                ..Default::default()
            };
            match info {
                Some(Info::Linked { percent, charging }) => {
                    b.percent = Some(percent);
                    b.charging = charging;
                }
                _ => b.asleep = true,
            }
            if keyboard {
                cache.insert(c.path.clone(), (Instant::now() + KEYBOARD_EVERY, b.clone()));
            }
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(data: &[u8]) -> Vec<u8> {
        let mut r = vec![REPORT, 0x01, 0x01, data.len() as u8, 0x00, 0x09, 0x00, 0x00];
        r.extend_from_slice(data);
        r.push(xor(&r[2..]));
        r.resize(64, 0);
        r
    }

    #[test]
    fn request_checksum() {
        let r = request(64);
        assert_eq!(
            &r[..9],
            &[0x4D, 0x01, 0x01, 0x00, 0x00, 0x09, 0x00, 0x00, 0x08]
        );
    }

    #[test]
    fn mouse_info() {
        let mut d = [0u8; 14];
        d[..4].copy_from_slice(&[0x37, 0x38, 0x14, 0x10]);
        d[10] = 2;
        d[11] = 1;
        d[12] = 25;
        assert_eq!(
            parse(&frame(&d), false),
            Some(Info::Linked {
                percent: 25,
                charging: true
            })
        );
        let mut bad = frame(&d);
        bad[DATA + 14] ^= 1;
        assert_eq!(parse(&bad, false), None);
        d[10] = 0;
        assert_eq!(parse(&frame(&d), false), Some(Info::Unlinked));
    }

    #[test]
    fn receiver_without_device() {
        let r = frame(&[0xFF]);
        assert_eq!(parse(&r, false), Some(Info::Unlinked));
        assert_eq!(parse(&r, true), Some(Info::Unlinked));
        assert_eq!(parse(&frame(&[0x00, 0x01]), true), None);
    }

    #[test]
    fn keyboard_needs_its_model() {
        let mut d = [0u8; 20];
        d[..4].copy_from_slice(&[0x37, 0x38, 0x23, 0x20]);
        d[10] = 2;
        d[12] = 70;
        assert_eq!(
            parse(&frame(&d), true),
            Some(Info::Linked {
                percent: 70,
                charging: false
            })
        );
        d[2] = 0x24;
        assert_eq!(parse(&frame(&d), true), None);
    }
}
