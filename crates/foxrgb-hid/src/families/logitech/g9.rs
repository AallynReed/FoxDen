//! Logitech G9 and G9x mouse LED, through the HID++ 1.0 LED colour register.

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, ack, at};
use crate::registry::{Detector, Info};

/// Short SET_REGISTER of register 0x57 to the corded device.
pub fn set_color(c: Color) -> [u8; 7] {
    [0x10, 0xFF, 0x80, 0x57, c.0, c.1, c.2]
}

pub struct G9 {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<Color>,
}

impl G9 {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(&id, name, DeviceKind::Mouse, [0.70, 0.62, 0.06, 0.22], 1, 1);
        G9 {
            hid,
            device,
            sent: None,
        }
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(G9::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G9 {
    fn name(&self) -> &str {
        "logitech g9"
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
        ack(self.hid.as_ref(), &set_color(c), 50)?;
        self.sent = Some(c);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    #[test]
    fn register_write_once_per_colour() {
        assert_eq!(set_color(Color(1, 2, 3)), [0x10, 0xFF, 0x80, 0x57, 1, 2, 3]);
        let (hid, log) = Fake::silent();
        let mut m = G9::new(Box::new(hid), "m".into(), "G9");
        m.push("m", &[Color(1, 2, 3)]).unwrap();
        m.push("m", &[Color(1, 2, 3)]).unwrap();
        assert_eq!(take(&log), vec![set_color(Color(1, 2, 3)).to_vec()]);
    }
}
