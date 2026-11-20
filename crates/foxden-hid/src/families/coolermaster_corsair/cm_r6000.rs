//! Ported from OpenRGB Controllers/CoolerMasterController/CMR6000Controller (GPL-2.0-or-later).

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, exchange, report, rgb, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x2516;
const RX6000: u16 = 0x014D;
const RX6900: u16 = 0x015B;
const TIMEOUT: i32 = 250;
const DIRECT: u8 = 0x00;

pub static DETECTORS: &[Detector] = &[
    det!(
        "Cooler Master Radeon RX 6000 GPU",
        VID,
        RX6000,
        Some(1),
        None,
        None,
        open
    ),
    det!(
        "Cooler Master Radeon RX 6900 GPU",
        VID,
        RX6900,
        Some(1),
        None,
        None,
        open
    ),
];

pub type Report = [u8; 65];

/// SendEnableCommand
pub fn enable() -> Report {
    report(&[0x41, 0x80])
}

/// SendSecondColour (RX 6900 only): primary then secondary colour.
pub fn second_colour(primary: Color, secondary: Color) -> Report {
    let mut r = report(&[0x51, 0x9C, 0x01, 0x00]);
    r[5..8].copy_from_slice(&rgb(primary));
    r[8..11].copy_from_slice(&rgb(secondary));
    r
}

/// The mode packet of SendUpdate for Direct: speed 0xFF, random 0, full brightness.
pub fn direct_mode(c: Color) -> Report {
    let mut r = [0xFF; 65];
    r[..8].copy_from_slice(&[0x00, 0x51, 0x2C, 0x01, 0x00, DIRECT, 0xFF, 0x00]);
    r[10] = 0xFF;
    r[11..14].copy_from_slice(&rgb(c));
    r[14..17].fill(0x00);
    r
}

/// SendColourConfig
pub fn colour_config(mode: u8) -> Report {
    let mut r = report(&[0x51, 0xA0, 0x01, 0x00, 0x00, 0x03, 0x00, 0x00, 0x05, 0x06]);
    r[0x0B..0x1A].fill(mode);
    r
}

/// SendApplyCommand
pub fn apply() -> Report {
    report(&[0x51, 0x28, 0x00, 0x00, 0xE0])
}

struct Gpu {
    hid: HidDevice,
    pid: u16,
}

impl Driver for Gpu {
    fn frame(&mut self, colors: &[Color], _first: bool) -> Result<(), String> {
        exchange(&self.hid, &enable(), TIMEOUT)?;
        if self.pid == RX6900 {
            exchange(&self.hid, &second_colour(colors[0], Color::BLACK), TIMEOUT)?;
        }
        write(&self.hid, &direct_mode(colors[0]))?;
        exchange(&self.hid, &colour_config(DIRECT), TIMEOUT)?;
        exchange(&self.hid, &apply(), TIMEOUT).map(|_| ())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Gpu,
        vec![[0.5, 0.5]],
        [1, 1],
    );
    let driver = Gpu { hid, pid: info.pid };
    Ok(Lights::boxed("cooler master gpu", dev, None, driver))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMR6000Controller::SendEnableCommand and SendApplyCommand
    #[test]
    fn enable_and_apply() {
        assert_eq!(enable(), report(&[0x41, 0x80]));
        assert_eq!(apply(), report(&[0x51, 0x28, 0x00, 0x00, 0xE0]));
    }

    // CMR6000Controller::SendUpdate mode packet for Direct
    #[test]
    fn direct_mode_bytes() {
        let r = direct_mode(Color(1, 2, 3));
        assert_eq!(
            &r[..17],
            &[
                0x00, 0x51, 0x2C, 0x01, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 1, 2, 3, 0, 0, 0
            ]
        );
        assert!(r[17..].iter().all(|&b| b == 0xFF));
    }

    // CMR6000Controller::SendColourConfig
    #[test]
    fn colour_config_bytes() {
        let r = colour_config(DIRECT);
        assert_eq!(
            &r[..11],
            &[
                0x00, 0x51, 0xA0, 0x01, 0x00, 0x00, 0x03, 0x00, 0x00, 0x05, 0x06
            ]
        );
        assert!(r[11..].iter().all(|&b| b == 0));
        assert!(colour_config(2)[0x0B..0x1A].iter().all(|&b| b == 2));
    }

    // CMR6000Controller::SendSecondColour
    #[test]
    fn second_colour_bytes() {
        let r = second_colour(Color(1, 2, 3), Color(4, 5, 6));
        assert_eq!(
            &r[..12],
            &[0x00, 0x51, 0x9C, 0x01, 0x00, 1, 2, 3, 4, 5, 6, 0]
        );
    }
}
