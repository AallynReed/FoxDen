//! Ported from OpenRGB Controllers/LogitechController/LogitechG213Controller (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, ack, at};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 300;
/// Left, middle, right, arrows and home keys, numpad.
const ZONES: [u8; 5] = [1, 2, 3, 4, 5];

/// SetDirect: one zone in one colour.
pub fn direct(zone: u8, c: Color) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..10].copy_from_slice(&[0x11, 0xFF, 0x0C, 0x3A, zone, 0x01, c.0, c.1, c.2, 0x02]);
    r
}

pub struct G213 {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<Vec<Color>>,
}

impl G213 {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(
            &id,
            name,
            DeviceKind::Keyboard,
            [0.06, 0.58, 0.54, 0.32],
            ZONES.len() as u32,
            1,
        );
        G213 {
            hid,
            device,
            sent: None,
        }
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(G213::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G213 {
    fn name(&self) -> &str {
        "logitech g213"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        self.sent = None;
        for (i, &zone) in ZONES.iter().enumerate() {
            ack(self.hid.as_ref(), &direct(zone, at(colors, i)), TIMEOUT)?;
        }
        self.sent = Some(colors.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    // LogitechG213Controller::SetDirect.
    #[test]
    fn direct_matches_openrgb() {
        assert_eq!(
            direct(3, Color(0x10, 0x20, 0x30)),
            [
                0x11, 0xFF, 0x0C, 0x3A, 0x03, 0x01, 0x10, 0x20, 0x30, 0x02, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0
            ]
        );
    }

    // RGBController_LogitechG213::DeviceUpdateLEDs sends led values 1..5 in order; Direct needs no setup.
    #[test]
    fn push_sends_each_zone_once() {
        let (hid, log) = Fake::silent();
        let mut kb = G213::new(Box::new(hid), "k".into(), "G213");
        let colors: Vec<_> = (0..5).map(|i| Color(i, 0, 0)).collect();
        kb.push("k", &colors).unwrap();
        let sent = take(&log);
        assert_eq!(sent.len(), 5);
        for (i, r) in sent.iter().enumerate() {
            assert_eq!(r[..], direct(i as u8 + 1, Color(i as u8, 0, 0))[..]);
        }
        kb.push("k", &colors).unwrap();
        assert!(take(&log).is_empty());
    }
}
