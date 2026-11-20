use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::util::hex4;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3537;
const CABLE: u16 = 0x100E;
const PIDS: [u16; 3] = [CABLE, 0x103E, 0x1050];
/// GetDeviceInfo: output report 07, length 3, command 0A, sub 01.
const DEVICE_INFO: [u8; 4] = [0x07, 0x03, 0x0A, 0x01];

#[derive(Debug, PartialEq)]
struct Info {
    address: [u8; 6],
    percent: Option<u8>,
    charging: bool,
}

/// Input report `06 [len] 0A 01 [id] [fw fw] [mac x6] [battery] [type]`; battery FF = charging.
fn parse_info(r: &[u8]) -> Option<Info> {
    if r.len() < 15 || r[0] != 0x06 || r[2] != 0x0A || r[3] != 0x01 {
        return None;
    }
    if r[1] < 14 || r[1] as usize + 1 > r.len() {
        return None;
    }
    let address = r[7..13].try_into().ok()?;
    let charging = r[13] == 0xFF;
    Some(Info {
        address,
        percent: (!charging).then(|| r[13].min(100)),
        charging,
    })
}

fn key_base(info: &Info, pid: u16) -> String {
    if info.address.iter().all(|&b| b == 0) || info.address.iter().all(|&b| b == 0xFF) {
        return format!("gamesir-{}:{}", hex4(VID), hex4(pid));
    }
    let mac: String = info.address.iter().map(|b| format!("{b:02X}")).collect();
    format!("gamesir-sapjoy-{mac}")
}

fn device_info(d: &Dev) -> Option<Info> {
    d.flush();
    if !d.write(&DEVICE_INFO) {
        return None;
    }
    let end = Instant::now() + Duration::from_millis(900);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Some(r) = d.read(left.min(Duration::from_millis(200)))
            && let Some(info) = parse_info(&r)
        {
            return Some(info);
        }
    }
    None
}

pub fn poll_game_sir_sapjoy(ctx: &mut Ctx) {
    let first = ctx.out.len();
    for pid in PIDS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.page == 0xFFA0 && c.out_len == 32 && c.in_len == 64 && c.feat_len == 0
        });
        for c in colls {
            let Some(info) = Dev::open(&c).and_then(|d| device_info(&d)) else {
                continue;
            };
            let base = key_base(&info, pid);
            if ctx.out[first..].iter().any(|o| o.key == base) {
                ctx.claim(VID, pid);
                break;
            }
            let b = Battery {
                key: ctx.stable_key(&base),
                name: if c.product.is_empty() {
                    "GameSir controller".into()
                } else {
                    c.product.clone()
                },
                via: if pid == CABLE {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Gamepad,
                percent: info.percent,
                charging: info.charging,
                ..Battery::default()
            };
            ctx.add(b, VID, pid);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(battery: u8, mac: [u8; 6]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..5].copy_from_slice(&[0x06, 14, 0x0A, 0x01, 0x02]);
        r[7..13].copy_from_slice(&mac);
        r[13] = battery;
        r
    }

    #[test]
    fn device_info_battery() {
        let mac = [0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6];
        let i = parse_info(&reply(0x60, mac)).unwrap();
        assert_eq!(
            i,
            Info {
                address: mac,
                percent: Some(96),
                charging: false
            }
        );
        assert_eq!(key_base(&i, 0x103E), "gamesir-sapjoy-A1B2C3D4E5F6");
        assert_eq!(parse_info(&reply(0xFF, mac)).unwrap().percent, None);
        assert_eq!(parse_info(&reply(130, mac)).unwrap().percent, Some(100));
        let blank = parse_info(&reply(50, [0; 6])).unwrap();
        assert_eq!(key_base(&blank, CABLE), "gamesir-3537:100E");
        let mut bad = reply(50, mac);
        bad[2] = 0x0B;
        assert_eq!(parse_info(&bad), None);
    }
}
