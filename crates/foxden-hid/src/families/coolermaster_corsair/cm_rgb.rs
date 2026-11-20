//! Ported from OpenRGB Controllers/CoolerMasterController/CMRGBController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, exchange, line, report, rgb};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 250;
const MULTIPLE: u8 = 0x04;
const BREATHING: u8 = 0x01;
const COLOR_CYCLE: u8 = 0x02;
const STAR: u8 = 0x03;
const OFF: u8 = 0xFE;

pub static DETECTORS: &[Detector] = &[det!(
    "Cooler Master RGB",
    0x2516,
    0x004F,
    Some(1),
    Some(0xFF00),
    Some(1),
    open
)];

pub type Report = [u8; 65];

/// SendFlowControl
pub fn flow(flag: u8) -> Report {
    report(&[0x41, flag])
}

/// SendApply
pub fn apply() -> Report {
    report(&[0x50, 0x55])
}

/// SendSetMode
pub fn set_mode(mode: u8) -> Report {
    report(&[0x51, 0x28, 0x00, 0x00, mode])
}

/// SendCustomColorStart
pub fn custom_start() -> Report {
    report(&[0x51, 0x30])
}

/// SendSetCustomColors: one colour per port.
pub fn custom_colors(c: [Color; 4]) -> Report {
    let mut r = report(&[0x51, 0xA8]);
    for (i, color) in c.iter().enumerate() {
        r[5 + i * 3..8 + i * 3].copy_from_slice(&rgb(*color));
    }
    r
}

/// SendSetConfig, full form.
pub fn set_config(mode: u8, speed: u8, brightness: u8, c1: Color, c2: Color) -> Report {
    let (brightness, c1, c2) = match mode {
        COLOR_CYCLE => (0xDF, Color(0xFF, 0xFF, 0xFF), Color::BLACK),
        OFF => (0x03, c1, c2),
        _ => (brightness, c1, c2),
    };
    let mut r = report(&[0x51, 0x2C, 0x00, 0x00, mode, speed]);
    r[7] = if mode == BREATHING { 0x20 } else { 0x00 };
    r[8] = if mode == STAR { 0x19 } else { 0xFF };
    r[9] = 0xFF;
    r[10] = brightness;
    r[11..14].copy_from_slice(&rgb(c1));
    r[14..17].copy_from_slice(&rgb(c2));
    r[17..].fill(0xFF);
    r
}

/// SetMode for the per-LED Custom mode: speed 0, brightness 0xFF, no mode colours.
pub fn custom_mode() -> [Report; 5] {
    [
        flow(0x01),
        set_config(MULTIPLE, 0x00, 0xFF, Color::BLACK, Color::BLACK),
        set_mode(MULTIPLE),
        apply(),
        flow(0x00),
    ]
}

/// SetLedsDirect
pub fn direct(c: [Color; 4]) -> [Report; 8] {
    [
        flow(0x80),
        custom_start(),
        custom_colors(c),
        set_mode(MULTIPLE),
        custom_start(),
        set_config(MULTIPLE, 0x00, 0xFF, c[0], Color::BLACK),
        apply(),
        flow(0x00),
    ]
}

struct Rgb {
    hid: HidDevice,
}

impl Driver for Rgb {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            for r in custom_mode() {
                exchange(&self.hid, &r, TIMEOUT)?;
            }
        }
        for r in direct([colors[0], colors[1], colors[2], colors[3]]) {
            exchange(&self.hid, &r, TIMEOUT)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Strip,
        line(4),
        [4, 1],
    );
    Ok(Lights::boxed("cooler master rgb", dev, None, Rgb { hid }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMRGBController::SendFlowControl, SendApply, SendSetMode and SendCustomColorStart
    #[test]
    fn small_packets() {
        assert_eq!(&flow(0x80)[..3], &[0x00, 0x41, 0x80]);
        assert_eq!(&apply()[..3], &[0x00, 0x50, 0x55]);
        assert_eq!(&set_mode(4)[..6], &[0x00, 0x51, 0x28, 0x00, 0x00, 0x04]);
        assert_eq!(&custom_start()[..4], &[0x00, 0x51, 0x30, 0x00]);
    }

    // CMRGBController::SendSetCustomColors
    #[test]
    fn custom_colors_bytes() {
        let r = custom_colors([
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ]);
        assert_eq!(
            &r[..18],
            &[
                0x00, 0x51, 0xA8, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0
            ]
        );
    }

    // CMRGBController::SendSetConfig(CM_RGBC_MODE_MULTIPLE, 0x00, 0xFF, color_1, 0, false)
    #[test]
    fn config_bytes() {
        let r = set_config(MULTIPLE, 0, 0xFF, Color(1, 2, 3), Color::BLACK);
        assert_eq!(
            &r[..17],
            &[
                0x00, 0x51, 0x2C, 0x00, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 1, 2, 3, 0, 0, 0
            ]
        );
        assert!(r[17..].iter().all(|&b| b == 0xFF));
        let b = set_config(BREATHING, 0x3C, 0x80, Color::BLACK, Color::BLACK);
        assert_eq!((b[7], b[8], b[10]), (0x20, 0xFF, 0x80));
        let s = set_config(STAR, 0x46, 0x80, Color::BLACK, Color::BLACK);
        assert_eq!(s[8], 0x19);
        let cyc = set_config(COLOR_CYCLE, 0x96, 0x10, Color::BLACK, Color(5, 5, 5));
        assert_eq!(&cyc[10..17], &[0xDF, 0xFF, 0xFF, 0xFF, 0, 0, 0]);
    }

    // CMRGBController::SetMode and SetLedsDirect sequences
    #[test]
    fn sequences() {
        let c = [Color(1, 2, 3), Color::BLACK, Color::BLACK, Color::BLACK];
        let d = direct(c);
        assert_eq!(d[0], flow(0x80));
        assert_eq!(d[2], custom_colors(c));
        assert_eq!(d[3], set_mode(4));
        assert_eq!(d[5][11..14], [1, 2, 3]);
        assert_eq!(d[7], flow(0x00));
        let m = custom_mode();
        assert_eq!(m[0], flow(0x01));
        assert_eq!(m[1][11..14], [0, 0, 0]);
        assert_eq!(m[4], flow(0x00));
    }
}
