//! Ported from OpenRGB Controllers/LogitechController/LogitechX56Controller (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, at};
use crate::registry::{Detector, Info};

/// Direct mode's default brightness.
const BRIGHTNESS: u8 = 0x64;

/// SetColor: a brightness report, then a colour report.
pub fn set_color(c: Color, brightness: u8) -> [[u8; 64]; 2] {
    let mut b = [0u8; 64];
    b[..4].copy_from_slice(&[0x09, 0x00, 0x02, brightness]);
    let mut c2 = [0u8; 64];
    c2[..6].copy_from_slice(&[0x09, 0x00, 0x03, c.0, c.1, c.2]);
    [b, c2]
}

pub struct X56 {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<Color>,
}

impl X56 {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(&id, name, DeviceKind::Other, [0.82, 0.60, 0.08, 0.10], 1, 1);
        X56 {
            hid,
            device,
            sent: None,
        }
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(X56::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for X56 {
    fn name(&self) -> &str {
        "logitech x56"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let c = at(colors, 0);
        if self.sent == Some(c) {
            return Ok(());
        }
        self.sent = None;
        for r in set_color(c, BRIGHTNESS) {
            self.hid.feature(&r)?;
        }
        self.sent = Some(c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    // LogitechX56Controller::SetColor at Direct's default brightness 0x64.
    #[test]
    fn set_color_matches_openrgb() {
        let [b, c] = set_color(Color(1, 2, 3), BRIGHTNESS);
        assert_eq!(b[..5], [0x09, 0x00, 0x02, 0x64, 0x00]);
        assert_eq!(c[..7], [0x09, 0x00, 0x03, 1, 2, 3, 0x00]);
        assert!(b[4..].iter().chain(&c[6..]).all(|&x| x == 0));
    }

    #[test]
    fn push_skips_repeats() {
        let (hid, log) = Fake::silent();
        let mut j = X56::new(Box::new(hid), "j".into(), "X56");
        j.push("j", &[Color(1, 2, 3)]).unwrap();
        j.push("j", &[Color(1, 2, 3)]).unwrap();
        assert_eq!(take(&log).len(), 2);
    }
}
