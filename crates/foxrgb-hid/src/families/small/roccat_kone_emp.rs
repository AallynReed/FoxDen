//! Roccat Kone EMP mouse LEDs, kept in the active profile's settings.

use std::time::{Duration, Instant};

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "roccat-kone-emp";
const CONFIGURE: u8 = 0x04;
const PROFILE: u8 = 0x05;
const SETTINGS: u8 = 0x06;
const SETTINGS_LEN: usize = 41;
const MAGIC: u8 = 0x29;
const LEDS: usize = 4;
/// Every change is a flash write, so changes land at most this often.
const GAP: Duration = Duration::from_millis(1000);

/// Settings with every LED on, fixed, no colour flow, each in a user colour; checksum redone.
pub fn recolor(settings: &[u8], colors: &[Color]) -> Vec<u8> {
    let mut b = settings[..SETTINGS_LEN].to_vec();
    b[20] = 0xFF;
    b[21] = 0x00;
    b[22] = 0x01;
    for (i, c) in colors.iter().take(LEDS).enumerate() {
        b[23 + i * 4..27 + i * 4].copy_from_slice(&[0x1E, c.0, c.1, c.2]);
    }
    let sum = b[..SETTINGS_LEN - 2]
        .iter()
        .fold(0u16, |s, &x| s.wrapping_add(x as u16));
    b[SETTINGS_LEN - 2..].copy_from_slice(&sum.to_le_bytes());
    b
}

fn get(io: &mut dyn Io, id: u8, len: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; len];
    b[0] = id;
    if io.get_feature(&mut b)? < len {
        return Err(format!("short Roccat report {id:02x}"));
    }
    Ok(b)
}

fn wait_ready(io: &mut dyn Io) -> Result<(), String> {
    io.sleep(10);
    for _ in 0..10 {
        match get(io, CONFIGURE, 3)?[1] {
            0x01 => return Ok(()),
            0x03 => io.sleep(100),
            _ => io.sleep(10),
        }
    }
    Err("Roccat mouse not ready".into())
}

struct KoneEmp {
    written: Option<Vec<Color>>,
    at: Option<Instant>,
}

impl Proto for KoneEmp {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if self.written.as_deref() == Some(colors) || self.at.is_some_and(|t| t.elapsed() < GAP) {
            return Ok(());
        }
        let profile = get(io, PROFILE, 3)?[2];
        io.feature(&[CONFIGURE, profile, 0x80])?;
        wait_ready(io)?;
        let settings = get(io, SETTINGS, SETTINGS_LEN)?;
        if settings[1] != MAGIC {
            return Err("unexpected Roccat settings".into());
        }
        io.feature(&recolor(&settings, colors))?;
        wait_ready(io)?;
        self.written = Some(colors.to_vec());
        self.at = Some(Instant::now());
        Ok(())
    }

    fn keepalive_ms(&self) -> Option<u64> {
        Some(GAP.as_millis() as u64)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    get(io.as_mut(), PROFILE, 3)?;
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, LEDS);
    let proto = KoneEmp {
        written: None,
        at: None,
    };
    Ok(Box::new(Driver::new(io, dev, FAMILY, proto)))
}

pub static DETECTORS: &[Detector] = &[det("Roccat Kone EMP", 0x1E7D, 0x2E24, ANY, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn settings() -> Vec<u8> {
        let mut s = vec![0u8; SETTINGS_LEN];
        s[0] = SETTINGS;
        s[1] = MAGIC;
        s[2] = 2;
        s[7] = 0x07;
        s[20] = 0xF0;
        s[22] = 0x03;
        s
    }

    #[test]
    fn leds_and_checksum() {
        let c = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        let b = recolor(&settings(), &c);
        assert_eq!(b[..3], [0x06, 0x29, 2]);
        assert_eq!(b[7], 0x07);
        assert_eq!(b[20..27], [0xFF, 0x00, 0x01, 0x1E, 1, 2, 3]);
        assert_eq!(b[35..39], [0x1E, 10, 11, 12]);
        let sum: u16 = b[..39].iter().map(|&x| x as u16).sum();
        assert_eq!(b[39..], sum.to_le_bytes());
    }

    #[test]
    fn reads_active_profile_then_writes_settings() {
        let replies = vec![
            vec![PROFILE, 0x03, 2],
            vec![CONFIGURE, 0x01, 0],
            settings(),
            vec![CONFIGURE, 0x01, 0],
        ];
        let mut rec = Rec::with(replies);
        let mut k = KoneEmp {
            written: None,
            at: None,
        };
        let c = [Color(9, 9, 9); 4];
        k.frame(&mut rec, &c).unwrap();
        let ops = rec.take();
        assert_eq!(ops[0], Op::GetF(PROFILE));
        assert_eq!(ops[1], Op::F(vec![CONFIGURE, 2, 0x80]));
        assert_eq!(ops[4], Op::GetF(SETTINGS));
        assert_eq!(ops[5], Op::F(recolor(&settings(), &c)));
        assert_eq!(ops.len(), 8);
    }
}
