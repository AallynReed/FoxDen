//! Ported from OpenRGB Controllers/LogitechController/LogitechGLightsyncController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::{FAMILY, Hid, ack, at, open_sibling};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 300;
const DEV_INDEX: u8 = 0xFF;
const FEATURE: u8 = 0x0E;
const STATIC: u8 = 0x01;

/// Zone count and function byte the detector passes for each mouse.
pub fn params(pid: u16) -> (usize, u8) {
    match pid {
        0xC080 | 0xC08F => (2, 0x3A),
        0xC085 | 0xC08C => (1, 0x3C),
        _ => (1, 0x3A),
    }
}

/// SetDirectMode(true), a short report on the usage-1 collection.
pub fn direct_mode() -> [u8; 7] {
    [0x10, DEV_INDEX, FEATURE, 0x8A, 0x01, 0x01, 0x00]
}

/// UpdateMouseLED with Direct mapped to Static, as DeviceUpdateZoneLEDs does.
pub fn zone(function: u8, zone: u8, c: Color) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..10].copy_from_slice(&[
        0x11, DEV_INDEX, FEATURE, function, zone, STATIC, c.0, c.1, c.2, 0x02,
    ]);
    r
}

pub struct Lightsync {
    short: Box<dyn Hid>,
    long: Box<dyn Hid>,
    function: u8,
    device: Device,
    sent: Option<Vec<Color>>,
}

impl Lightsync {
    fn new(short: Box<dyn Hid>, long: Box<dyn Hid>, pid: u16, id: String, name: &str) -> Self {
        let (zones, function) = params(pid);
        // two zones are DPI then logo, front to back
        let device = Device::grid(
            &id,
            name,
            DeviceKind::Mouse,
            [0.70, 0.62, 0.06, 0.22],
            1,
            zones as u32,
        );
        Lightsync {
            short,
            long,
            function,
            device,
            sent: None,
        }
    }

    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        if self.sent.is_none() {
            self.short.write(&direct_mode())?;
            let mut buf = [0u8; 64];
            self.long.read(&mut buf, TIMEOUT)?;
        }
        for i in 0..self.device.leds.len() {
            ack(
                self.long.as_ref(),
                &zone(self.function, i as u8, at(colors, i)),
                TIMEOUT,
            )?;
        }
        Ok(())
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let short = open_sibling(api, info, info.usage_page, 1)?;
    let long = open_sibling(api, info, info.usage_page, 2)?;
    Ok(Box::new(Lightsync::new(
        Box::new(short),
        Box::new(long),
        info.pid,
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for Lightsync {
    fn name(&self) -> &str {
        "logitech lightsync"
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
    use super::super::fake::{Fake, Log, take};
    use super::*;

    // LogitechGLightsyncController::SetDirectMode(true).
    #[test]
    fn direct_mode_matches_openrgb() {
        assert_eq!(direct_mode(), [0x10, 0xFF, 0x0E, 0x8A, 0x01, 0x01, 0x00]);
    }

    // LogitechGLightsyncController::UpdateMouseLED for Direct (sent as Static, speed 0).
    #[test]
    fn zone_matches_openrgb() {
        assert_eq!(
            zone(0x3C, 0, Color(1, 2, 3))[..11],
            [
                0x11, 0xFF, 0x0E, 0x3C, 0x00, 0x01, 0x01, 0x02, 0x03, 0x02, 0x00
            ]
        );
        assert!(zone(0x3A, 1, Color(1, 2, 3))[10..].iter().all(|&b| b == 0));
    }

    // DetectLogitechMouseG203/G303/G403/GPRO arguments.
    #[test]
    fn params_match_detectors() {
        assert_eq!(params(0xC084), (1, 0x3A));
        assert_eq!(params(0xC080), (2, 0x3A));
        assert_eq!(params(0xC08F), (2, 0x3A));
        assert_eq!(params(0xC085), (1, 0x3C));
        assert_eq!(params(0xC08C), (1, 0x3C));
    }

    // DeviceUpdateMode for Direct: SetDirectMode, then DeviceUpdateLEDs zone 0 and zone 1.
    #[test]
    fn push_takes_control_once() {
        let log = Log::default();
        let short = Fake::new(log.clone(), |_| Vec::new());
        let long = Fake::new(log.clone(), |_| Vec::new());
        let mut m = Lightsync::new(Box::new(short), Box::new(long), 0xC080, "m".into(), "G303");
        let colors = [Color(1, 1, 1), Color(2, 2, 2)];
        m.push("m", &colors).unwrap();
        assert_eq!(
            take(&log),
            vec![
                direct_mode().to_vec(),
                zone(0x3A, 0, colors[0]).to_vec(),
                zone(0x3A, 1, colors[1]).to_vec()
            ]
        );
        m.push("m", &colors).unwrap();
        assert!(take(&log).is_empty());
        m.push("m", &[Color(3, 3, 3), Color(2, 2, 2)]).unwrap();
        assert_eq!(take(&log).len(), 2);
    }
}
