use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::vendor_frames::evision_query;
use crate::hid::{Coll, Dev};
use crate::{Battery, Ctx, Kind};

const EVISION: u16 = 0x320F;
const RECEIVER: u16 = 0x5088;
const CABLE: u16 = 0x5055;
const Z87_VID: u16 = 0x373B;
const Z87_RECEIVER: u16 = 0x1012;
/// Asking a receiver more often keeps the keyboard awake.
const RECEIVER_EVERY: Duration = Duration::from_secs(600);

/// Product strings of receivers and cables, and whether the battery request carries sub-command 06.
const MODELS: &[(&str, bool)] = &[
    ("ATK V75X 2.4G Dongle", false),
    ("ATK V75X", false),
    ("VGN S99 2.4G Dongle", true),
    ("VGN S99", true),
    ("VGN V98pro V2 2.4G Dongle", true),
    ("VGN V98pro V2", true),
    ("VGN V98 PRO 2.4G Dongle", true),
    ("VGN V98pro", true),
    ("VGN N75 PRO 2.4G Dongle", true),
    ("VGN N75 PRO", true),
    ("VGN VXE75 V2 2.4G Dongle", true),
    ("VGN VXE75 V2", true),
    ("VGN VXE75 2.4G Dongle", true),
    ("VGN VXE75", true),
    ("VXE V87 2.4G Dongle", true),
    ("VXE V87", true),
    ("VXE V87 RGB", true),
    ("Wireless Gaming Keyboard", false),
];

fn model(product: &str) -> Option<(&'static str, bool)> {
    let p = product.trim_end_matches(' ');
    MODELS
        .iter()
        .copied()
        .find(|(m, _)| m.eq_ignore_ascii_case(p))
}

fn keyboard_name(product: &str) -> &str {
    const SUFFIX: &str = " 2.4G Dongle";
    let n = product.len();
    if n > SUFFIX.len() && product.is_char_boundary(n - SUFFIX.len()) {
        let (head, tail) = product.split_at(n - SUFFIX.len());
        if tail.eq_ignore_ascii_case(SUFFIX) {
            return head;
        }
    }
    product
}

#[derive(Default)]
struct Cache {
    asked: Option<Instant>,
    level: Option<(u8, bool)>,
}

#[derive(Default)]
struct Z87Cache {
    next: Option<Instant>,
    level: Option<(u8, bool)>,
}

static EVISION_CACHE: Mutex<Cache> = Mutex::new(Cache {
    asked: None,
    level: None,
});
static Z87_CACHE: Mutex<Z87Cache> = Mutex::new(Z87Cache {
    next: None,
    level: None,
});

fn ask(c: &Coll, sub: u8, timeout: Duration, busy: bool) -> Option<(u8, bool)> {
    evision_query(&Dev::open(c)?, sub, timeout, busy)
}

fn poll_z87(ctx: &mut Ctx) {
    if ctx.is_claimed(Z87_VID, Z87_RECEIVER) {
        return;
    }
    let mut cache = Z87_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let found = ctx
        .find(Z87_VID, Z87_RECEIVER, |c| {
            c.page == 0xFF1C && c.usage == 0x0092 && c.in_len == 64 && c.out_len == 64
        })
        .into_iter()
        .find(|c| {
            c.product
                .trim_end_matches(' ')
                .eq_ignore_ascii_case("ATK Z87 Dongle")
        });
    let Some(c) = found else {
        *cache = Z87Cache::default();
        return;
    };
    let now = Instant::now();
    if cache.next.is_none_or(|t| now >= t) {
        cache.level = ask(&c, 0x00, Duration::from_millis(1000), true);
        cache.next = Some(
            now + if cache.level.is_some() {
                RECEIVER_EVERY
            } else {
                Duration::from_secs(30)
            },
        );
    }
    let b = Battery {
        key: ctx.stable_key(&format!("atk-{Z87_VID:04X}:{Z87_RECEIVER:04X}")),
        name: "ATK Z87".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Keyboard,
        percent: cache.level.map(|l| l.0),
        charging: cache.level.is_some_and(|l| l.1),
        asleep: cache.level.is_none(),
        grace_secs: 1800,
        ..Default::default()
    };
    ctx.add(b, Z87_VID, Z87_RECEIVER);
}

pub fn poll_atk_keyboard(ctx: &mut Ctx) {
    poll_z87(ctx);
    for pid in [CABLE, RECEIVER] {
        if ctx.is_claimed(EVISION, pid) {
            continue;
        }
        let cable = pid == CABLE;
        let colls = ctx.find(EVISION, pid, |c| c.page == 0xFF1C && c.usage == 0x0092);
        let Some((c, (product, vgn))) =
            colls.iter().find_map(|c| model(&c.product).map(|m| (c, m)))
        else {
            continue;
        };
        let mut cache = EVISION_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let due = cable
            || cache
                .asked
                .is_none_or(|t| now.duration_since(t) >= RECEIVER_EVERY);
        let level = if due {
            cache.asked = Some(now);
            let fresh = ask(
                c,
                if vgn { 0x06 } else { 0x00 },
                Duration::from_millis(2500),
                false,
            );
            if fresh.is_some() {
                cache.level = fresh;
            }
            fresh
        } else {
            cache.level
        };
        let b = Battery {
            key: ctx.stable_key("atk-evision-keyboard"),
            name: keyboard_name(product).into(),
            via: if cable {
                "USB cable"
            } else {
                "2.4 GHz receiver"
            }
            .into(),
            kind: Kind::Keyboard,
            percent: level.map(|l| l.0),
            charging: level.is_some_and(|l| l.1),
            asleep: level.is_none(),
            grace_secs: 1800,
            ..Default::default()
        };
        ctx.add(b, EVISION, pid);
        if cable {
            ctx.claim(EVISION, RECEIVER);
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_names() {
        assert_eq!(
            model("VGN S99 2.4G Dongle  "),
            Some(("VGN S99 2.4G Dongle", true))
        );
        assert_eq!(model("atk v75x"), Some(("ATK V75X", false)));
        assert_eq!(model("Some Other Keyboard"), None);
        assert_eq!(keyboard_name("VGN S99 2.4G Dongle"), "VGN S99");
        assert_eq!(keyboard_name("VXE V87 RGB"), "VXE V87 RGB");
    }
}
