//! Ported from OpenRGB Controllers/RazerController/RazerKrakenV3Controller (GPL-2.0-or-later).

use super::{model, stacked};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device};
use hidapi::{HidApi, HidDevice};

pub const REPORT_LEN: usize = 15;
const REPORT_ID: u8 = 0x40;
const CMD_SET_MODE: u8 = 0x01;
const CMD_SET_BRIGHTNESS: u8 = 0x02;
const CMD_SET_COLOR: u8 = 0x03;
const MODE_DIRECT: u8 = 0x08;

pub type Report = [u8; REPORT_LEN];

/// `razer_kraken_create_v3_report` with the command and arguments filled in.
fn report(cmd: u8, args: &[(usize, u8)]) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[0] = REPORT_ID;
    r[1] = cmd;
    for &(i, v) in args {
        r[2 + i] = v;
    }
    r
}

/// `SetModeDirect`.
pub fn set_mode_direct() -> Report {
    report(CMD_SET_MODE, &[(1, 0x0F), (2, MODE_DIRECT)])
}

/// `SetBrightness`.
pub fn set_brightness(level: u8) -> Report {
    report(CMD_SET_BRIGHTNESS, &[(2, level)])
}

/// `SetDirect`: one RGB triple per table column.
pub fn set_direct(led_count: usize, colors: &[Color]) -> Report {
    let mut r = report(CMD_SET_COLOR, &[]);
    for i in 0..led_count.min(4) {
        let c = colors.get(i).copied().unwrap_or(Color::BLACK);
        r[3 + i * 3..6 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    r
}

/// The first push runs `DeviceUpdateMode(Direct)`: mode, then full brightness.
pub fn plan(first: bool, led_count: usize, colors: &[Color]) -> Vec<Report> {
    let mut out = Vec::new();
    if first {
        out.push(set_mode_direct());
        out.push(set_brightness(255));
    }
    out.push(set_direct(led_count, colors));
    out
}

pub struct KrakenV3 {
    hid: HidDevice,
    leds: usize,
    device: Device,
    sent: Option<Vec<Color>>,
}

pub fn open(api: &HidApi, info: &Info, detector: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let m = model(info.pid);
    Ok(Box::new(KrakenV3 {
        hid,
        leds: m.cols as usize,
        device: stacked(
            info.device_id("razer"),
            detector.name.into(),
            m.kind,
            m.zones,
        ),
        sent: None,
    }))
}

impl Backend for KrakenV3 {
    fn name(&self) -> &str {
        "razer kraken v3"
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
        for r in plan(self.sent.is_none(), self.leds, colors) {
            if let Err(e) = self.hid.write(&r) {
                self.sent = None;
                return Err(e.to_string());
            }
        }
        self.sent = Some(colors.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RazerKrakenV3Controller::SetModeDirect, matching its "4001000f08..." capture.
    #[test]
    fn direct_mode_report() {
        assert_eq!(
            set_mode_direct(),
            [0x40, 0x01, 0x00, 0x0F, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    // RazerKrakenV3Controller::SetBrightness.
    #[test]
    fn brightness_report() {
        assert_eq!(
            set_brightness(0xFF),
            [0x40, 0x02, 0x00, 0x00, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    // RazerKrakenV3Controller::SetDirect for one LED and for the Kitty V2 Pro's four.
    #[test]
    fn direct_frames() {
        assert_eq!(
            set_direct(1, &[Color(0xFF, 0xFF, 0xFF)]),
            [
                0x40, 0x03, 0x00, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        let four = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        assert_eq!(
            set_direct(4, &four),
            [0x40, 0x03, 0x00, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
    }

    // RGBController_RazerKrakenV3::DeviceUpdateMode(Direct) then DeviceUpdateLEDs.
    #[test]
    fn first_push_takes_control() {
        let p = plan(true, 1, &[Color(1, 2, 3)]);
        assert_eq!(
            p,
            vec![
                set_mode_direct(),
                set_brightness(255),
                set_direct(1, &[Color(1, 2, 3)])
            ]
        );
        assert_eq!(plan(false, 1, &[Color(1, 2, 3)]).len(), 1);
    }

    #[test]
    fn kitty_v2_pro_has_four_leds() {
        let m = model(0x0554);
        assert_eq!(m.cols, 4);
        assert_eq!(m.led_count(), 4);
    }
}
