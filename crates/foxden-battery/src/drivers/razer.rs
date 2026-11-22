use std::thread::sleep;
use std::time::Duration;

use crate::hid::Dev;
use crate::util::{clamp100, guess_kind, kind_from_collections};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1532;
const LEN: usize = 90;
const POWER: u8 = 0x07;
const LEVEL: u8 = 0x80;
const CHARGING: u8 = 0x84;

const KEYBOARDS: &[u16] = &[
    0x010D, 0x010E, 0x010F, 0x0111, 0x0113, 0x0118, 0x011A, 0x011B, 0x011C, 0x0201, 0x0202, 0x0203,
    0x0204, 0x0205, 0x0207, 0x0208, 0x0209, 0x020F, 0x0210, 0x0211, 0x0214, 0x0216, 0x0217, 0x021A,
    0x021E, 0x021F, 0x0220, 0x0221, 0x0224, 0x0225, 0x0226, 0x0227, 0x0228, 0x022A, 0x022B, 0x022C,
    0x022D, 0x022F, 0x0232, 0x0233, 0x0234, 0x0235, 0x0237, 0x0239, 0x023A, 0x023B, 0x023F, 0x0240,
    0x0241, 0x0243, 0x0244, 0x0245, 0x0246, 0x024A, 0x024B, 0x024C, 0x024D, 0x024E, 0x0252, 0x0253,
    0x0255, 0x0256, 0x0257, 0x0258, 0x0259, 0x025A, 0x025C, 0x025D, 0x025E, 0x0266, 0x0268, 0x0269,
    0x026A, 0x026B, 0x026C, 0x026D, 0x026E, 0x026F, 0x0270, 0x0271, 0x0276, 0x0279, 0x027A, 0x0282,
    0x0287, 0x028A, 0x028B, 0x028C, 0x028D, 0x028F, 0x0290, 0x0292, 0x0293, 0x0294, 0x0295, 0x0296,
    0x0298, 0x029D, 0x029E, 0x029F, 0x02A0, 0x02A1, 0x02A2, 0x02A3, 0x02A5, 0x02A6, 0x02A7, 0x02B0,
    0x02B6, 0x02B8, 0x02B9, 0x02BA, 0x02C5, 0x02C6, 0x02C7, 0x02C9, 0x02CD, 0x02CF, 0x02D2, 0x02D4,
    0x02D5, 0x02D7, 0x02E0, 0x02E1, 0x02EA, 0x0A24,
];

const MICE: &[u16] = &[
    0x0013, 0x0015, 0x0016, 0x001F, 0x0020, 0x0024, 0x0025, 0x0029, 0x002E, 0x002F, 0x0032, 0x0034,
    0x0036, 0x0037, 0x0038, 0x0039, 0x003E, 0x003F, 0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045,
    0x0046, 0x0048, 0x004C, 0x004F, 0x0050, 0x0053, 0x0054, 0x0059, 0x005A, 0x005B, 0x005C, 0x005E,
    0x005F, 0x0060, 0x0062, 0x0064, 0x0065, 0x0067, 0x006A, 0x006B, 0x006C, 0x006E, 0x006F, 0x0070,
    0x0071, 0x0072, 0x0073, 0x0077, 0x0078, 0x007A, 0x007B, 0x007C, 0x007D, 0x0080, 0x0083, 0x0084,
    0x0085, 0x0086, 0x0088, 0x008A, 0x008C, 0x008D, 0x008F, 0x0090, 0x0091, 0x0094, 0x0095, 0x0096,
    0x0098, 0x0099, 0x009A, 0x009C, 0x009E, 0x009F, 0x00A1, 0x00A3, 0x00A5, 0x00A6, 0x00A7, 0x00A8,
    0x00AA, 0x00AB, 0x00AF, 0x00B0, 0x00B2, 0x00B3, 0x00B4, 0x00B6, 0x00B7, 0x00B8, 0x00B9, 0x00BE,
    0x00BF, 0x00C0, 0x00C1, 0x00C2, 0x00C3, 0x00C4, 0x00C5, 0x00C7, 0x00C8, 0x00CB, 0x00CC, 0x00CD,
    0x00D0, 0x00D1, 0x00D3, 0x00D4, 0x00D6, 0x00D7, 0x00DA, 0x00DB, 0x00DE, 0x00DF, 0x00E0, 0x00E2,
    0x00E3, 0x00E5, 0x00E6, 0x00E7, 0x00E8,
];

/// Relays to the docked mouse: 0x1F or 0x0F reach the mouse, 0xFF the dock itself.
const MOUSE_DOCK_PRO: u16 = 0x00A4;
/// Naga V3 Pro over its receiver answers about 100 ms after a request.
const SLOW: &[u16] = &[0x00E8];

/// Milliseconds to wait for the first answer and for each later look.
fn pace(pid: u16) -> (u64, u64) {
    if keyboard_tid(pid).is_some() {
        (5, 5)
    } else if SLOW.contains(&pid) {
        (100, 100)
    } else {
        (15, 40)
    }
}

fn tids(pid: u16) -> Vec<u8> {
    match (pid, keyboard_tid(pid)) {
        (MOUSE_DOCK_PRO, _) => vec![0x1F, 0x0F],
        _ if SLOW.contains(&pid) => vec![0x1F],
        (_, Some(t)) => vec![t],
        _ => vec![0x1F, 0x3F, 0xFF, 0x08, 0x9F],
    }
}

/// The transaction id a keyboard's battery commands need, when it is known.
fn keyboard_tid(pid: u16) -> Option<u8> {
    match pid {
        0x025C | 0x0271 | 0x0290 | 0x0296 | 0x02BA | 0x02C9 | 0x02D2 | 0x02D5 => Some(0x9F),
        0x0258 | 0x0292 | 0x0298 | 0x02B9 | 0x02CD | 0x02D4 | 0x02D7 => Some(0x1F),
        0x025A => Some(0x3F),
        _ => None,
    }
}

fn crc(r: &[u8]) -> u8 {
    r[2..88].iter().fold(0, |a, b| a ^ b)
}

/// 90-byte report: status, tid, remaining (2), protocol, size, class, id, 80 args, crc, reserved.
fn request(tid: u8, class: u8, id: u8, size: u8) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[1] = tid;
    r[5] = size;
    r[6] = class;
    r[7] = id;
    r[88] = crc(&r);
    r
}

#[derive(Debug, PartialEq)]
enum Status {
    Busy,
    Done(Vec<u8>),
    Offline,
    Unsupported,
}

/// The device's answer to `req`; `None` when the mailbox holds someone else's reply.
fn status(req: &[u8; LEN], r: &[u8]) -> Option<Status> {
    if r.len() < LEN
        || r[1] != req[1]
        || r[2..6] != [0, 0, 0, req[5]]
        || r[6..8] != req[6..8]
        || r[88] != crc(r)
    {
        return None;
    }
    match r[0] {
        0x01 => Some(Status::Busy),
        0x02 => Some(Status::Done(r[8..88].to_vec())),
        0x04 => Some(Status::Offline),
        0x03 | 0x05 => Some(Status::Unsupported),
        _ => None,
    }
}

#[derive(PartialEq)]
enum Outcome {
    Args(Vec<u8>),
    Offline,
    Unsupported,
    Silent,
}

/// The feature mailbox is shared with lighting and the vendor app: resend when someone else's reply is there.
fn command(dev: &Dev, tid: u8, class: u8, id: u8, (first, next): (u64, u64)) -> Outcome {
    let req = request(tid, class, id, 0x02);
    let mut out = vec![0u8];
    out.extend_from_slice(&req);
    let mut send = true;
    let mut last = Outcome::Silent;
    for attempt in 0..8 {
        if send && !dev.set_feature(&out) {
            return Outcome::Silent;
        }
        sleep(Duration::from_millis(if attempt == 0 {
            first
        } else {
            next
        }));
        send = true;
        last = Outcome::Silent;
        let Some(r) = dev.get_feature(0) else {
            continue;
        };
        match r.get(1..).and_then(|r| status(&req, r)) {
            Some(Status::Busy) => send = false,
            Some(Status::Done(args)) => return Outcome::Args(args),
            Some(Status::Offline) => last = Outcome::Offline,
            Some(Status::Unsupported) => return Outcome::Unsupported,
            None => {}
        }
    }
    last
}

fn clean_name(n: &str) -> String {
    let mut n = n.trim().to_string();
    for suffix in [
        " HyperSpeed Dongle",
        " Dongle",
        " (Wired)",
        " (Wireless)",
        " Wireless Receiver",
        " Receiver",
    ] {
        if n.len() > suffix.len()
            && let Some(s) = n.strip_suffix(suffix)
        {
            n = s.to_string();
        }
    }
    if n.is_empty() {
        "Razer device".into()
    } else {
        n
    }
}

fn level_percent(raw: u8) -> u8 {
    clamp100((raw as i32 * 100 + 127) / 255)
}

pub fn poll_razer(ctx: &mut Ctx) {
    let first = ctx.out.len();
    for (vid, pid) in ctx.products(VID) {
        for c in ctx.find(vid, pid, |c| c.feat_len as usize == LEN + 1) {
            let Some(dev) = Dev::open_feature_only(&c) else {
                continue;
            };
            let pace = pace(pid);
            let mut found = None;
            let mut offline = false;
            for t in tids(pid) {
                match command(&dev, t, POWER, LEVEL, pace) {
                    Outcome::Args(a) => {
                        found = Some((t, a));
                        break;
                    }
                    Outcome::Offline => offline = true,
                    _ => {}
                }
            }
            if found.is_none() && !offline {
                continue;
            }
            let docked = pid == MOUSE_DOCK_PRO;
            let name = if docked {
                "Razer mouse on Mouse Dock Pro".to_string()
            } else {
                clean_name(&c.product)
            };
            let mut kind = if KEYBOARDS.contains(&pid) {
                Kind::Keyboard
            } else if docked || MICE.contains(&pid) {
                Kind::Mouse
            } else {
                kind_from_collections(&ctx.colls, vid, pid)
            };
            if kind == Kind::Other {
                kind = guess_kind(&name);
            }
            let mut b = Battery {
                name,
                via: if docked {
                    "Mouse Dock Pro"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind,
                grace_secs: 120,
                connected: found.is_some(),
                ..Battery::default()
            };
            if let Some((tid, args)) = found {
                b.percent = Some(level_percent(args[1]));
                if let Outcome::Args(ch) = command(&dev, tid, POWER, CHARGING, pace) {
                    b.charging = ch[1] == 1;
                }
            }
            if let Some(o) = ctx.out[first..].iter_mut().find(|o| o.name == b.name) {
                if !o.connected && b.connected {
                    o.connected = true;
                    o.percent = b.percent;
                }
                o.charging |= b.charging;
                ctx.claim(vid, pid);
            } else {
                b.key = ctx.stable_key(&format!("razer-1532:{pid:04X}"));
                ctx.add(b, vid, pid);
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(req: &[u8; LEN], code: u8, args: &[u8]) -> [u8; LEN] {
        let mut r = *req;
        r[0] = code;
        r[8..8 + args.len()].copy_from_slice(args);
        r[88] = crc(&r);
        r
    }

    #[test]
    fn request_layout() {
        let r = request(0x1F, POWER, LEVEL, 0x02);
        assert_eq!(r[..8], [0x00, 0x1F, 0x00, 0x00, 0x00, 0x02, 0x07, 0x80]);
        assert_eq!(r[88], 0x02 ^ 0x07 ^ 0x80);
    }

    #[test]
    fn replies_are_matched_to_the_request() {
        let req = request(0x1F, POWER, LEVEL, 0x02);
        assert_eq!(
            status(&req, &reply(&req, 0x02, &[0x00, 0xFF]))
                .map(|s| matches!(s, Status::Done(a) if a[1] == 0xFF)),
            Some(true)
        );
        assert_eq!(status(&req, &reply(&req, 0x01, &[])), Some(Status::Busy));
        assert_eq!(status(&req, &reply(&req, 0x04, &[])), Some(Status::Offline));
        assert_eq!(
            status(&req, &reply(&req, 0x05, &[])),
            Some(Status::Unsupported)
        );
        let lighting = request(0x1F, 0x0F, 0x02, 0x09);
        assert_eq!(status(&req, &reply(&lighting, 0x02, &[])), None);
        let mut bad = reply(&req, 0x02, &[0, 0x80]);
        bad[88] ^= 1;
        assert_eq!(status(&req, &bad), None);
    }

    #[test]
    fn levels_names_and_kinds() {
        assert_eq!(level_percent(255), 100);
        assert_eq!(level_percent(128), 50);
        assert_eq!(level_percent(0), 0);
        assert_eq!(
            clean_name("Razer Viper Ultimate Dongle"),
            "Razer Viper Ultimate"
        );
        assert_eq!(
            clean_name("Razer DeathAdder V3 Pro (Wired)"),
            "Razer DeathAdder V3 Pro"
        );
        assert_eq!(clean_name(""), "Razer device");
        assert_eq!(keyboard_tid(0x025C), Some(0x9F));
        assert!(MICE.contains(&0x007A) && KEYBOARDS.contains(&0x0A24));
        assert!(
            [0x00E2, 0x00E3, 0x00E7, 0x00E8]
                .iter()
                .all(|p| MICE.contains(p))
        );
        assert!(MICE.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn dock_and_slow_receivers() {
        assert_eq!(tids(MOUSE_DOCK_PRO), [0x1F, 0x0F]);
        assert_eq!(tids(0x00E3)[0], 0x1F);
        assert_eq!(tids(0x025C), [0x9F]);
        assert_eq!(pace(0x00E8), (100, 100));
        assert_eq!(tids(0x00E8), [0x1F]);
        assert_eq!(pace(0x00E3), (15, 40));
        let r = request(0x0F, POWER, CHARGING, 0x02);
        assert_eq!(r[..8], [0x00, 0x0F, 0x00, 0x00, 0x00, 0x02, 0x07, 0x84]);
        assert_eq!(r[88], 0x02 ^ 0x07 ^ 0x84);
    }
}
