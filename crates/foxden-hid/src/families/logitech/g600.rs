//! Ported from OpenRGB Controllers/LogitechController/LogitechG600Controller (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, at};
use crate::registry::{Detector, Info};

const DIRECT: u8 = 0x00;

/// SetMode feature report for Direct, whose speed is 0.
pub fn direct(c: Color) -> [u8; 8] {
    [0xF1, c.0, c.1, c.2, DIRECT, 0x00, 0x00, 0x00]
}

pub struct G600 {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<Color>,
}

impl G600 {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(&id, name, DeviceKind::Mouse, [0.70, 0.62, 0.06, 0.22], 1, 1);
        G600 {
            hid,
            device,
            sent: None,
        }
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(G600::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G600 {
    fn name(&self) -> &str {
        "logitech g600"
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
        self.hid.feature(&direct(c))?;
        self.sent = Some(c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    // LogitechG600Controller::SetMode with Direct's value and speed.
    #[test]
    fn direct_matches_openrgb() {
        assert_eq!(direct(Color(1, 2, 3)), [0xF1, 1, 2, 3, 0, 0, 0, 0]);
    }

    #[test]
    fn push_skips_repeats() {
        let (hid, log) = Fake::silent();
        let mut m = G600::new(Box::new(hid), "m".into(), "G600");
        m.push("m", &[Color(1, 2, 3)]).unwrap();
        m.push("m", &[Color(1, 2, 3)]).unwrap();
        assert_eq!(take(&log), vec![direct(Color(1, 2, 3)).to_vec()]);
    }
}
