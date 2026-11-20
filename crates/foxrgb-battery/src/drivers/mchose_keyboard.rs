use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3837;
const PID: u16 = 0x3033;
const REQ_FLAG: u8 = 0x55;
const REPLY_FLAG: u8 = 0xAA;
const FUNC_CONFIG: u8 = 0x05;
const SIZE: u8 = 56;
const PAYLOAD: usize = 8;
const BATTERY: usize = 32;
/// The keyboard sleeps on its own schedule; asking rarely keeps from waking it.
const ASK_EVERY: Duration = Duration::from_secs(600);

/// Read request: checksum is size + offset (reads carry no payload).
fn request(size: u8) -> [u8; 9] {
    [
        0x00,
        REQ_FLAG,
        FUNC_CONFIG,
        0x00,
        size,
        size,
        0x00,
        0x00,
        0x00,
    ]
}

/// (percent, charging) from a func-config reply at offset 0.
fn parse(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < PAYLOAD + BATTERY + 1
        || r[0] != REPLY_FLAG
        || r[1] != FUNC_CONFIG
        || r[5] != 0
        || r[6] != 0
    {
        return None;
    }
    let size = r[4] as usize;
    if size < BATTERY + 1 || PAYLOAD + size > r.len() {
        return None;
    }
    let sum = r[4..PAYLOAD + size]
        .iter()
        .fold(0u8, |a, b| a.wrapping_add(*b));
    if sum != r[3] {
        return None;
    }
    let b = r[PAYLOAD + BATTERY];
    let pct = b & 0x7F;
    (pct <= 100).then_some((pct, b & 0x80 != 0))
}

fn query(dev: &Dev) -> Option<(u8, bool)> {
    dev.flush();
    for attempt in 0..2 {
        if attempt > 0 {
            sleep(Duration::from_millis(150));
        }
        if dev.write(&request(SIZE))
            && let Some(v) = dev.read_until(Duration::from_millis(500), parse)
        {
            return Some(v);
        }
    }
    None
}

#[derive(Default)]
struct Last {
    asked: Option<Instant>,
    reading: Option<(u8, bool)>,
}

static LAST: Mutex<Last> = Mutex::new(Last {
    asked: None,
    reading: None,
});

fn display_name(product: &str) -> String {
    match product
        .strip_suffix(" 2.4G")
        .filter(|n| !n.is_empty())
        .unwrap_or(product)
    {
        "" => "MCHOSE keyboard".into(),
        n => n.into(),
    }
}

pub fn poll_mchose_keyboard(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.in_len == 65
            && c.out_len == 65
            && matches!((c.page, c.usage), (0x0001, 0x0000) | (0xFF31, 0x0074))
    });
    let Some(first) = colls.first() else { return };

    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let due = last.asked.is_none_or(|t| t.elapsed() >= ASK_EVERY);
    let reading = if due {
        last.asked = Some(Instant::now());
        let got = colls
            .iter()
            .find_map(|c| Dev::open(c).and_then(|d| query(&d)));
        if got.is_some() {
            last.reading = got;
        }
        got
    } else {
        last.reading
    };
    // Never answered: another protocol behind this receiver, or no keyboard.
    if last.reading.is_none() {
        return;
    }

    let mut b = Battery {
        key: ctx.stable_key("mchose-3837:3033"),
        name: display_name(&first.product),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Keyboard,
        grace_secs: 1800,
        ..Default::default()
    };
    match reading {
        Some((pct, charging)) => {
            b.percent = Some(pct);
            b.charging = charging;
        }
        None => b.asleep = true,
    }
    ctx.add(b, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_request() {
        assert_eq!(request(56), [0x00, 0x55, 0x05, 0x00, 56, 56, 0, 0, 0]);
    }

    #[test]
    fn func_config_battery() {
        let mut r = vec![0u8; 64];
        r[..8].copy_from_slice(&[0xAA, 0x05, 0x00, 0x00, 56, 0, 0, 0]);
        r[PAYLOAD + BATTERY] = 85;
        r[PAYLOAD + 1] = 7;
        r[3] = (56u32 + 85 + 7) as u8;
        assert_eq!(parse(&r), Some((85, false)));
        r[PAYLOAD + BATTERY] = 0x80 | 60;
        assert_eq!(parse(&r), None);
        r[3] = (56u32 + 0xBC + 7) as u8;
        assert_eq!(parse(&r), Some((60, true)));
    }
}
