use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::util::interpolate;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x046D;
const PID: u16 = 0x0ADF;
const OTA: u8 = 0x06;
const WIRELESS: u8 = 0x0E;
const GET_MODE: u8 = 0xE0;
const SET_MODE: u8 = 0xE1;
const SESSION_INIT: [u8; 19] = [
    0x50, 0x41, 0x06, 0x01, 0x0D, 0x00, 0x25, 0x34, 0x12, 0x5A, 0x5A, 0x01, 0x00, 0x00, 0x00, 0xF0,
    0x00, 0x00, 0x00,
];

/// CRC-16/XMODEM.
fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |mut crc, &b| {
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

/// `cmd, body length, crc lo, crc hi, body`, zero-padded to 64; the crc covers the frame with its field zeroed.
fn frame(cmd: u8, body: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[0] = cmd;
    f[1] = body.len() as u8;
    let n = body.len().min(60);
    f[4..4 + n].copy_from_slice(&body[..n]);
    let crc = crc16(&f).to_le_bytes();
    f[2..4].copy_from_slice(&crc);
    f
}

/// A command for the headset's radio chip: `50 41 category seq len payload`.
fn pa(category: u8, seq: u8, len: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x50, 0x41, category, seq, len];
    v.extend_from_slice(payload);
    v
}

/// Millivolts from a battery reply `00 07 00 06 seq 00 lo hi` anywhere in the report.
fn millivolts(r: &[u8]) -> Option<u16> {
    r.windows(8).find_map(|w| {
        let mv = u16::from_le_bytes([w[6], w[7]]);
        (w[..4] == [0x00, 0x07, 0x00, 0x06] && w[5] == 0 && (2500..=4500).contains(&mv))
            .then_some(mv)
    })
}

const CURVE: &[(i32, i32)] = &[
    (4200, 100),
    (4150, 95),
    (4100, 90),
    (4050, 85),
    (4000, 80),
    (3950, 75),
    (3900, 70),
    (3850, 62),
    (3800, 55),
    (3750, 45),
    (3700, 35),
    (3650, 25),
    (3600, 15),
    (3550, 10),
    (3500, 5),
    (3400, 0),
];

/// Sends one frame and listens for `wait`; returns millivolts as soon as a battery reply shows up.
fn exchange(dev: &Dev, cmd: u8, body: &[u8], wait: Duration) -> Option<Option<u16>> {
    let mut out = vec![0u8];
    out.extend_from_slice(&frame(cmd, body));
    dev.flush();
    if !dev.write(&out) {
        return None;
    }
    let end = Instant::now() + wait;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Some(r) = dev.read(left)
            && let Some(mv) = millivolts(&r)
        {
            return Some(Some(mv));
        }
    }
    Some(None)
}

/// An open session; closing it on drop.
struct Session<'a> {
    dev: &'a Dev,
    seq: u8,
}

impl Session<'_> {
    fn pa(&mut self, category: u8, len: u8, payload: &[u8], wait: Duration) -> Option<u16> {
        let cmd = pa(category, self.seq, len, payload);
        self.seq = self.seq % 127 + 1;
        exchange(self.dev, 0x80, &cmd, wait).flatten()
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        exchange(self.dev, 0x85, &[0x02], Duration::from_millis(50));
    }
}

/// Opens a session, routes the query to the headset rather than the dongle, reads the battery, closes.
fn read_millivolts(dev: &Dev) -> Option<u16> {
    let ms = Duration::from_millis;
    exchange(dev, 0x84, &[0x01], ms(50))?;
    let mut s = Session { dev, seq: 2 };
    exchange(dev, 0x85, &[0x01], ms(50));
    exchange(dev, 0x80, &SESSION_INIT, ms(450));
    s.pa(WIRELESS, 1, &[GET_MODE], ms(120));
    s.pa(WIRELESS, 2, &[SET_MODE, 0x01], ms(120));
    s.pa(WIRELESS, 1, &[GET_MODE], ms(120));
    s.pa(OTA, 1, &[0x00, 0x31], ms(250))
}

pub fn poll_logitech_g435(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.page == 0xFF00 && c.usage == 0x0001 && c.in_len == 65 && c.out_len == 65
    });
    let Some(dev) = colls.first().and_then(Dev::open) else {
        return;
    };
    let mv = read_millivolts(&dev);
    let b = Battery {
        key: ctx.stable_key("logitech-046D:0ADF"),
        name: "Logitech G435".into(),
        via: "2.4 GHz receiver".into(),
        kind: Kind::Headset,
        percent: mv.map(|mv| interpolate(mv as i32, CURVE)),
        connected: mv.is_some(),
        ..Battery::default()
    };
    ctx.add(b, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_is_xmodem() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn frames_carry_length_and_crc() {
        let f = frame(0x84, &[0x01]);
        assert_eq!(f[..2], [0x84, 0x01]);
        assert_eq!(f[4], 0x01);
        let mut zeroed = f;
        zeroed[2] = 0;
        zeroed[3] = 0;
        assert_eq!(u16::from_le_bytes([f[2], f[3]]), crc16(&zeroed));
        assert_eq!(
            pa(OTA, 5, 1, &[0x00, 0x31]),
            [0x50, 0x41, 0x06, 0x05, 0x01, 0x00, 0x31]
        );
    }

    #[test]
    fn battery_reply_is_found_in_the_report() {
        let mut r = vec![0u8; 64];
        r[10..18].copy_from_slice(&[0x00, 0x07, 0x00, 0x06, 0x07, 0x00, 0xD0, 0x0F]);
        assert_eq!(millivolts(&r), Some(4048));
        r[16..18].copy_from_slice(&[0x10, 0x00]);
        assert_eq!(millivolts(&r), None);
        assert_eq!(interpolate(4048, CURVE), 84);
        assert_eq!(interpolate(3300, CURVE), 0);
    }
}
