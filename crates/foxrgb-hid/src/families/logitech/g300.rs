//! Logitech G300 mouse LED: one bit per channel, kept in the active onboard profile.

use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{FAMILY, at};
use crate::registry::{Detector, Info};

const ACTIVE: u8 = 0xF0;
const PROFILE_0: u8 = 0xF3;
const PROFILE_LEN: usize = 35;
/// Each change is a flash write, so changes land at most this often.
const GAP: Duration = Duration::from_millis(1000);

/// The three LED bits a colour rounds to.
pub fn bits(c: Color) -> u8 {
    u8::from(c.0 > 127) | u8::from(c.1 > 127) << 1 | u8::from(c.2 > 127) << 2
}

/// Profile report id of the active profile, from the 4-byte 0xF0 report.
pub fn profile_id(active: &[u8]) -> Result<u8, String> {
    match active.get(1).map(|b| b >> 4) {
        Some(p @ 0..=2) => Ok(PROFILE_0 + p),
        _ => Err("no active G300 profile".into()),
    }
}

/// The profile report with only its LED bits changed.
pub fn recolor(profile: &[u8], bits: u8) -> Vec<u8> {
    let mut b = profile.to_vec();
    b[1] = (b[1] & !0x07) | bits;
    b
}

fn feature(hid: &HidDevice, id: u8, len: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; len];
    buf[0] = id;
    let n = hid
        .get_feature_report(&mut buf)
        .map_err(|e| e.to_string())?;
    if n < len {
        return Err(format!("short G300 report {id:02x}"));
    }
    Ok(buf)
}

pub struct G300 {
    hid: HidDevice,
    device: Device,
    sent: Option<u8>,
    at: Option<Instant>,
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    profile_id(&feature(&hid, ACTIVE, 4)?)?;
    let device = Device::grid(
        &info.device_id(FAMILY),
        det.name,
        DeviceKind::Mouse,
        [0.70, 0.62, 0.06, 0.22],
        1,
        1,
    );
    Ok(Box::new(G300 {
        hid,
        device,
        sent: None,
        at: None,
    }))
}

impl Backend for G300 {
    fn name(&self) -> &str {
        "logitech g300"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let want = bits(at(colors, 0));
        if self.sent == Some(want) || self.at.is_some_and(|t| t.elapsed() < GAP) {
            return Ok(());
        }
        let id = profile_id(&feature(&self.hid, ACTIVE, 4)?)?;
        let profile = feature(&self.hid, id, PROFILE_LEN)?;
        if profile[1] & 0x07 != want {
            self.hid
                .send_feature_report(&recolor(&profile, want))
                .map_err(|e| e.to_string())?;
            self.at = Some(Instant::now());
        }
        self.sent = Some(want);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn led_bits_in_the_active_profile() {
        assert_eq!(bits(Color(200, 0, 255)), 0b101);
        assert_eq!(bits(Color(127, 128, 0)), 0b010);
        assert_eq!(profile_id(&[0xF0, 0x2A, 0, 0]), Ok(0xF5));
        assert!(profile_id(&[0xF0, 0x30, 0, 0]).is_err());
        let mut p = vec![0u8; PROFILE_LEN];
        p[0] = 0xF4;
        p[1] = 0xFD;
        p[2] = 0x03;
        let r = recolor(&p, 0b011);
        assert_eq!(r[..3], [0xF4, 0xFB, 0x03]);
        assert_eq!(r.len(), 35);
    }
}
