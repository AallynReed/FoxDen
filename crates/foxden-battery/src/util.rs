use crate::Kind;
use crate::hid::Coll;

pub fn clamp100(v: i32) -> u8 {
    v.clamp(0, 100) as u8
}

/// Linear map of `lo..=hi` onto 0..=100, clamped.
pub fn map_to_100(v: i32, lo: i32, hi: i32) -> u8 {
    if hi == lo {
        return 0;
    }
    clamp100((v - lo) * 100 / (hi - lo))
}

/// Piecewise-linear percentage through `(millivolts, percent)` points sorted by falling voltage.
pub fn interpolate(mv: i32, table: &[(i32, i32)]) -> u8 {
    let Some(&(top_mv, top_pct)) = table.first() else {
        return 0;
    };
    if mv >= top_mv {
        return clamp100(top_pct);
    }
    for w in table.windows(2) {
        let ((hi_mv, hi_pct), (lo_mv, lo_pct)) = (w[0], w[1]);
        if mv >= lo_mv {
            let span = (hi_mv - lo_mv).max(1);
            return clamp100(lo_pct + (mv - lo_mv) * (hi_pct - lo_pct) / span);
        }
    }
    clamp100(table.last().unwrap().1)
}

/// CRC-16 with polynomial 0x1021, MSB first: `init` 0 is XMODEM, 0xFFFF is CCITT-FALSE.
pub fn crc16_ccitt(init: u16, data: &[u8]) -> u16 {
    data.iter().fold(init, |mut crc, &b| {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// Wrapping byte sum.
pub fn sum8(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |a, &v| a.wrapping_add(v))
}

pub fn xor8(b: &[u8]) -> u8 {
    b.iter().fold(0, |a, v| a ^ v)
}

pub fn hex4(v: u16) -> String {
    format!("{v:04X}")
}

/// Device type from words in its name.
pub fn guess_kind(name: &str) -> Kind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&[
        "headset",
        "headphone",
        "earbud",
        "buds",
        "airpods",
        "arctis",
        "blackshark",
        "cloud",
        "inzone",
    ]) {
        Kind::Headset
    } else if has(&["keyboard", "kbd"]) {
        Kind::Keyboard
    } else if has(&["mouse", "mice"]) {
        Kind::Mouse
    } else if has(&[
        "controller",
        "gamepad",
        "joystick",
        "dualsense",
        "dualshock",
        "xbox",
    ]) {
        Kind::Gamepad
    } else if has(&["mic"]) {
        Kind::Microphone
    } else {
        Kind::Other
    }
}

/// Device type from the generic desktop collections a product exposes: mouse, then gamepad, then keyboard.
pub fn kind_from_collections(all: &[Coll], vid: u16, pid: u16) -> Kind {
    let usages: Vec<u16> = all
        .iter()
        .filter(|c| c.vid == vid && c.pid == pid && c.page == 0x01)
        .map(|c| c.usage)
        .collect();
    if usages.contains(&0x02) {
        Kind::Mouse
    } else if usages.iter().any(|u| matches!(u, 0x04 | 0x05)) {
        Kind::Gamepad
    } else if usages.contains(&0x06) {
        Kind::Keyboard
    } else {
        Kind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voltage_curves_interpolate_between_points() {
        let t = [(4200, 100), (3700, 50), (3300, 0)];
        assert_eq!(interpolate(4300, &t), 100);
        assert_eq!(interpolate(3950, &t), 75);
        assert_eq!(interpolate(3500, &t), 25);
        assert_eq!(interpolate(3000, &t), 0);
        assert_eq!(map_to_100(15, 10, 20), 50);
    }
}
