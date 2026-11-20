//! Ported from OpenRGB Controllers/LogitechController/LogitechG203LController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, ack, at};
use crate::registry::{Detector, Info};

/// SendPacket's read timeout.
const TIMEOUT: i32 = 10;

/// The constructor's "enable software control" packet.
pub fn software_control() -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..7].copy_from_slice(&[0x11, 0xFF, 0x0E, 0x50, 0x01, 0x03, 0x07]);
    r
}

/// SetDevice: left, centre and right LED in one packet.
pub fn frame(colors: &[Color]) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..4].copy_from_slice(&[0x11, 0xFF, 0x12, 0x10]);
    for i in 0..3 {
        let c = at(colors, i);
        r[4 + i * 4..8 + i * 4].copy_from_slice(&[i as u8 + 1, c.0, c.1, c.2]);
    }
    r[0x10] = 0xFF;
    r
}

/// SendApply.
pub fn apply() -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..4].copy_from_slice(&[0x11, 0xFF, 0x12, 0x70]);
    r
}

pub struct G203L {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<Vec<Color>>,
}

impl G203L {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(&id, name, DeviceKind::Mouse, [0.70, 0.62, 0.06, 0.22], 3, 1);
        G203L {
            hid,
            device,
            sent: None,
        }
    }

    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let hid = self.hid.as_ref();
        if self.sent.is_none() {
            ack(hid, &software_control(), TIMEOUT)?;
        }
        // DeviceUpdateLEDs sends the frame twice, OpenRGB's workaround for colour lag
        for _ in 0..2 {
            ack(hid, &frame(colors), TIMEOUT)?;
            ack(hid, &apply(), TIMEOUT)?;
        }
        Ok(())
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(G203L::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G203L {
    fn name(&self) -> &str {
        "logitech g203 lightsync"
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
        match self.send(colors) {
            Ok(()) => {
                self.sent = Some(colors.to_vec());
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    // LogitechG203LController constructor.
    #[test]
    fn software_control_matches_openrgb() {
        assert_eq!(
            software_control()[..8],
            [0x11, 0xFF, 0x0E, 0x50, 0x01, 0x03, 0x07, 0x00]
        );
    }

    // LogitechG203LController::SetDevice and SendApply.
    #[test]
    fn frame_matches_openrgb() {
        let f = frame(&[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)]);
        assert_eq!(
            f,
            [
                0x11, 0xFF, 0x12, 0x10, 0x01, 1, 2, 3, 0x02, 4, 5, 6, 0x03, 7, 8, 9, 0xFF, 0, 0, 0
            ]
        );
        assert_eq!(apply()[..5], [0x11, 0xFF, 0x12, 0x70, 0x00]);
    }

    #[test]
    fn push_takes_control_once_and_sends_twice() {
        let (hid, log) = Fake::silent();
        let mut m = G203L::new(Box::new(hid), "m".into(), "G203");
        let colors = [Color(1, 2, 3); 3];
        m.push("m", &colors).unwrap();
        let f = frame(&colors).to_vec();
        let a = apply().to_vec();
        assert_eq!(
            take(&log),
            vec![
                software_control().to_vec(),
                f.clone(),
                a.clone(),
                f.clone(),
                a.clone()
            ]
        );
        m.push("m", &colors).unwrap();
        assert!(take(&log).is_empty());
    }
}
