//! Ported from OpenRGB Controllers/CoolerMasterController/CMMP750Controller (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, report, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x2516;
const PAGE: Option<u16> = Some(0xFF00);
const USAGE: Option<u16> = Some(1);

pub static DETECTORS: &[Detector] = &[
    det!(
        "Cooler Master MP750 XL",
        VID,
        0x0109,
        None,
        PAGE,
        USAGE,
        open
    ),
    det!(
        "Cooler Master MP750 Large",
        VID,
        0x0107,
        None,
        PAGE,
        USAGE,
        open
    ),
    det!(
        "Cooler Master MP750 Medium",
        VID,
        0x0105,
        None,
        PAGE,
        USAGE,
        open
    ),
];

const STATIC: u8 = 0x01;
const SPEEDS: [u8; 9] = [0xFF, 0xE0, 0xC0, 0xA0, 0x80, 0x60, 0x40, 0x20, 0x00];

/// SendUpdate in Static mode, the mode with a per-LED colour. Selecting it sets speed index 0.
pub fn static_color(c: Color) -> [u8; 65] {
    report(&[STATIC, 0x04, c.0, c.1, c.2, SPEEDS[0]])
}

struct Mp750 {
    hid: HidDevice,
}

impl Driver for Mp750 {
    fn frame(&mut self, colors: &[Color], _first: bool) -> Result<(), String> {
        write(&self.hid, &static_color(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Other,
        vec![[0.5, 0.5]],
        [1, 1],
    );
    Ok(Lights::boxed(
        "cooler master mp750",
        dev,
        None,
        Mp750 { hid },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMMP750Controller::SendUpdate after SetMode(Static, 0) and SetColor
    #[test]
    fn static_bytes() {
        let r = static_color(Color(0x11, 0x22, 0x33));
        assert_eq!(&r[..8], &[0x00, 0x01, 0x04, 0x11, 0x22, 0x33, 0xFF, 0x00]);
        assert!(r[7..].iter().all(|&b| b == 0));
    }
}
