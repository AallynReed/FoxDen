//! ASTRO A50 Gen 5 base station LED brightness.

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, at};
use crate::registry::{Detector, Info};

/// Brightness 0 to 100 from the brightest channel.
pub fn brightness(c: Color) -> u8 {
    (c.0.max(c.1).max(c.2) as u16 * 100).div_ceil(255) as u8
}

/// Report 0x02 frame: marker, length, command 0x0F with handle 0x1C, then the brightness.
pub fn frame(level: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..7].copy_from_slice(&[0x02, 0x0C, 0x04, 0x00, 0x0F, 0x1C, level]);
    b
}

pub struct A50 {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<u8>,
}

impl A50 {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(&id, name, DeviceKind::Other, [0.62, 0.30, 0.30, 0.20], 1, 1);
        A50 {
            hid,
            device,
            sent: None,
        }
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(A50::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for A50 {
    fn name(&self) -> &str {
        "astro a50"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let level = brightness(at(colors, 0));
        if self.sent == Some(level) {
            return Ok(());
        }
        self.sent = None;
        self.hid.write(&frame(level))?;
        self.sent = Some(level);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    #[test]
    fn brightness_frame() {
        assert_eq!(brightness(Color(255, 0, 0)), 100);
        assert_eq!(brightness(Color::BLACK), 0);
        assert_eq!(brightness(Color(0, 1, 0)), 1);
        let (hid, log) = Fake::silent();
        let mut a = A50::new(Box::new(hid), "a".into(), "A50");
        a.push("a", &[Color(0, 0, 255)]).unwrap();
        a.push("a", &[Color(0, 255, 0)]).unwrap();
        let sent = take(&log);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0][..8], [0x02, 0x0C, 0x04, 0x00, 0x0F, 0x1C, 100, 0]);
        assert_eq!(sent[0].len(), 64);
    }
}
