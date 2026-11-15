//! Ported from OpenRGB Controllers/LogitechController/LogitechG933Controller (GPL-2.0-or-later).

use std::thread::sleep;
use std::time::Duration;

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, at};
use crate::registry::{Detector, Info};

const RETRIES: u32 = 3;
const DIRECT: u8 = 0x01;
/// Logo, then the LED strip.
const ZONES: [u8; 2] = [0x00, 0x01];

/// SetDirectMode: resets the zone so it takes direct colours. OpenRGB leaves bytes 5.. unset; these are zero.
pub fn direct_mode(zone: u8) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..5].copy_from_slice(&[0x11, 0xFF, 0x04, 0xCA, zone]);
    r
}

/// SendHeadsetMode in Direct.
pub fn color(zone: u8, c: Color) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..10].copy_from_slice(&[0x11, 0xFF, 0x04, 0x3A, zone, DIRECT, c.0, c.1, c.2, 0x02]);
    r
}

/// fail_retry_write: the volume control shares this collection, so a write can fail and is retried.
fn retry_write(hid: &dyn Hid, data: &[u8]) {
    for _ in 0..RETRIES {
        sleep(Duration::from_millis(1));
        if hid.write(data).is_ok() {
            sleep(Duration::from_millis(1));
            let mut buf = [0u8; 64];
            let _ = hid.read(&mut buf, 20);
            return;
        }
        sleep(Duration::from_millis(10));
    }
}

pub struct G933 {
    hid: Box<dyn Hid>,
    device: Device,
    sent: Option<Vec<Color>>,
}

impl G933 {
    fn new(hid: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device::grid(&id, name, DeviceKind::Other, [0.82, 0.12, 0.08, 0.16], 1, 2);
        G933 {
            hid,
            device,
            sent: None,
        }
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(G933::new(
        Box::new(hid),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G933 {
    fn name(&self) -> &str {
        "logitech g933"
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
        let hid = self.hid.as_ref();
        if self.sent.is_none() {
            for zone in ZONES {
                retry_write(hid, &direct_mode(zone));
            }
        }
        for (i, zone) in ZONES.into_iter().enumerate() {
            retry_write(hid, &color(zone, at(colors, i)));
        }
        self.sent = Some(colors.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    // LogitechG933Controller::SetDirectMode.
    #[test]
    fn direct_mode_matches_openrgb() {
        assert_eq!(direct_mode(1)[..6], [0x11, 0xFF, 0x04, 0xCA, 0x01, 0x00]);
    }

    // LogitechG933Controller::SendHeadsetMode with LOGITECH_G933_MODE_DIRECT.
    #[test]
    fn color_matches_openrgb() {
        assert_eq!(
            color(0, Color(7, 8, 9)),
            [
                0x11, 0xFF, 0x04, 0x3A, 0x00, 0x01, 7, 8, 9, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    // RGBController_LogitechG933::DeviceUpdateMode: SetDirectMode per LED, then DeviceUpdateLEDs.
    #[test]
    fn push_resets_zones_once() {
        let (hid, log) = Fake::silent();
        let mut h = G933::new(Box::new(hid), "h".into(), "G933");
        let colors = [Color(1, 0, 0), Color(0, 1, 0)];
        h.push("h", &colors).unwrap();
        assert_eq!(
            take(&log),
            vec![
                direct_mode(0).to_vec(),
                direct_mode(1).to_vec(),
                color(0, colors[0]).to_vec(),
                color(1, colors[1]).to_vec()
            ]
        );
        h.push("h", &colors).unwrap();
        assert!(take(&log).is_empty());
    }
}
