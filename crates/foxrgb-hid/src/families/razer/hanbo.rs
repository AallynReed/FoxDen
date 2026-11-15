//! Ported from OpenRGB Controllers/RazerController/RazerHanboController (GPL-2.0-or-later).

use super::model;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device};
use hidapi::{HidApi, HidDevice};
use std::thread::sleep;
use std::time::Duration;

pub const REPORT_LEN: usize = 65;

pub type Report = [u8; REPORT_LEN];

/// `razer_hanbo_create_report` with arguments from index 0.
fn report(header: u8, args: &[u8]) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[1] = header;
    r[2..2 + args.len()].copy_from_slice(args);
    r
}

/// `SetDirectMode`, each write followed by a short read in the controller.
pub fn set_direct_mode() -> [Report; 4] {
    [
        report(0x82, &[0x01]),
        report(0x80, &[0x01, 0x00, 0x01]),
        report(0x80, &[0x01, 0x01, 0x01]),
        report(0x82, &[0x01]),
    ]
}

/// `SetZoneLeds`: zone 0 is the pump, then the fans; colours are G, R, B.
pub fn set_zone_leds(zone: u8, colors: &[Color]) -> Report {
    let mut args = vec![0x01, 0x07, 0x00, 0x00, 0x00, 0x00, zone];
    for c in colors.iter().take((63 - 7) / 3) {
        args.extend_from_slice(&[c.1, c.0, c.2]);
    }
    report(if zone > 0 { 0x40 } else { 0x32 }, &args)
}

/// `DeviceUpdateLEDs`: one report per zone, in LED order.
pub fn frame(zones: &[(u8, u8)], colors: &[Color]) -> Vec<Report> {
    let mut at = 0;
    let mut out = Vec::new();
    for (i, &(rows, cols)) in zones.iter().enumerate() {
        let n = rows as usize * cols as usize;
        let mut zone = vec![Color::BLACK; n];
        for (k, c) in zone.iter_mut().enumerate() {
            if let Some(v) = colors.get(at + k) {
                *c = *v;
            }
        }
        out.push(set_zone_leds(i as u8, &zone));
        at += n;
    }
    out
}

fn device(id: String, name: String) -> Device {
    let m = model(0x0F35);
    let rings = m.zones.len().max(1);
    let mut leds = Vec::new();
    for (z, &(rows, cols)) in m.zones.iter().enumerate() {
        let n = rows as usize * cols as usize;
        let cx = (z as f32 + 0.5) / rings as f32;
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            leds.push([cx + 0.4 / rings as f32 * a.sin(), 0.5 - 0.4 * a.cos()]);
        }
    }
    Device {
        id,
        name,
        kind: m.kind,
        rect: [0.30, 0.30, 0.40, 0.10],
        grid: [rings as u32 * 4, 4],
        leds,
    }
}

pub struct Hanbo {
    hid: HidDevice,
    zones: &'static [(u8, u8)],
    device: Device,
    sent: Option<Vec<Color>>,
}

pub fn open(api: &HidApi, info: &Info, detector: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(Hanbo {
        hid,
        zones: model(info.pid).zones,
        device: device(info.device_id("razer"), detector.name.into()),
        sent: None,
    }))
}

impl Hanbo {
    fn take_control(&self) -> Result<(), String> {
        let mut reply = [0u8; REPORT_LEN];
        for r in set_direct_mode() {
            self.hid.write(&r).map_err(|e| e.to_string())?;
            let _ = self.hid.read_timeout(&mut reply, 2);
        }
        Ok(())
    }

    fn paint(&self, colors: &[Color]) -> Result<(), String> {
        for r in frame(self.zones, colors) {
            self.hid.write(&r).map_err(|e| e.to_string())?;
            sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}

impl Backend for Hanbo {
    fn name(&self) -> &str {
        "razer hanbo"
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
        let result = if self.sent.is_none() {
            self.take_control().and_then(|()| self.paint(colors))
        } else {
            self.paint(colors)
        };
        self.sent = result.as_ref().ok().map(|()| colors.to_vec());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(header: u8, args: &[u8]) -> Report {
        let mut r = [0u8; 65];
        r[1] = header;
        r[2..2 + args.len()].copy_from_slice(args);
        r
    }

    // RazerHanboController::SetDirectMode: arguments carry over between the middle writes.
    #[test]
    fn direct_mode_sequence() {
        let s = set_direct_mode();
        assert_eq!(s[0], bytes(0x82, &[0x01]));
        assert_eq!(s[1], bytes(0x80, &[0x01, 0x00, 0x01]));
        assert_eq!(s[2], bytes(0x80, &[0x01, 0x01, 0x01]));
        assert_eq!(s[3], bytes(0x82, &[0x01]));
    }

    // RazerHanboController::SetZoneLeds for the pump and the third fan.
    #[test]
    fn zone_reports() {
        let pump = set_zone_leds(0, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(pump, bytes(0x32, &[1, 7, 0, 0, 0, 0, 0, 2, 1, 3, 5, 4, 6]));
        let fan = set_zone_leds(3, &[Color(0xAA, 0xBB, 0xCC); 18]);
        assert_eq!(fan[..9], [0x00, 0x40, 1, 7, 0, 0, 0, 0, 3]);
        assert_eq!(fan[9..12], [0xBB, 0xAA, 0xCC]);
        assert_eq!(fan[9 + 17 * 3..9 + 18 * 3], [0xBB, 0xAA, 0xCC]);
        assert_eq!(fan[63..], [0, 0]);
    }

    // RGBController_RazerHanbo::DeviceUpdateLEDs over the table's 16 + 3 x 18 LEDs.
    #[test]
    fn frame_splits_by_zone() {
        let m = model(0x0F35);
        assert_eq!(m.zones, &[(1, 16), (1, 18), (1, 18), (1, 18)]);
        let colors: Vec<Color> = (0..70).map(|i| Color(i, 0, 0)).collect();
        let f = frame(m.zones, &colors);
        assert_eq!(f.len(), 4);
        assert_eq!(f[0][1], 0x32);
        assert_eq!(f[0][9 + 15 * 3 + 1], 15);
        assert_eq!(f[0][9 + 16 * 3..], [0; 8]);
        assert_eq!(f[1][1], 0x40);
        assert_eq!(f[1][8], 1);
        assert_eq!(f[1][10], 16);
        assert_eq!(f[3][9 + 17 * 3 + 1], 69);
    }

    #[test]
    fn device_has_seventy_leds_in_bounds() {
        let d = device("x".into(), "X".into());
        assert_eq!(d.leds.len(), 70);
        assert!(d.leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }
}
