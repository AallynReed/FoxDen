//! Ported from OpenRGB Controllers/CorsairVoidController (GPL-2.0-or-later).

use std::time::{Duration, Instant};

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1B1C;
const PAGE: Option<u16> = Some(0xFFC5);
const USAGE: Option<u16> = Some(0x01);
/// A headset switched off and on returns to firmware lighting, so host control is asked for again.
const REFRESH: Duration = Duration::from_millis(1000);

pub static DETECTORS: &[Detector] = &[
    det!(
        "Corsair Void Pro Wireless",
        VID,
        0x0A14,
        None,
        PAGE,
        USAGE,
        open
    ),
    det!(
        "Corsair Void Pro Wireless",
        VID,
        0x0A1A,
        None,
        PAGE,
        USAGE,
        open
    ),
    det!("Corsair Void Pro USB", VID, 0x0A17, None, PAGE, USAGE, open),
    det!("Corsair Void Pro USB", VID, 0x0A1D, None, PAGE, USAGE, open),
    det!("Corsair Void Pro USB", VID, 0x0A16, None, PAGE, USAGE, open),
    det!("Corsair Void Pro", VID, 0x1B27, None, PAGE, USAGE, open),
    det!(
        "Corsair Void Pro Wireless",
        VID,
        0x1B2A,
        None,
        PAGE,
        USAGE,
        open
    ),
    det!("Corsair Void USB", VID, 0x1B23, None, PAGE, USAGE, open),
    det!("Corsair Void", VID, 0x1B29, None, PAGE, USAGE, open),
    det!("Corsair Void", VID, 0x0A2B, None, PAGE, USAGE, open),
    det!("Corsair Void Elite", VID, 0x0A51, None, PAGE, USAGE, open),
    det!("Corsair Void Elite", VID, 0x0A52, None, PAGE, USAGE, open),
    det!("Corsair Void Elite", VID, 0x0A55, None, PAGE, USAGE, open),
    det!("Corsair Void Elite", VID, 0x0A56, None, PAGE, USAGE, open),
    det!("Corsair Void Elite", VID, 0x0A75, None, PAGE, USAGE, open),
];

/// C8 01 hands lighting to the host, C8 00 gives it back to the headset.
pub fn lighting_control(host: bool) -> [u8; 3] {
    [0xC8, host as u8, 0x00]
}

/// Register write CB 06 with (register, value) pairs for both earcups.
pub fn set_colors(left: Color, right: Color) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..14].copy_from_slice(&[
        0xCB, 0x06, 0x1C, left.0, 0x16, left.1, 0x17, left.2, 0x1D, right.0, 0x18, right.1, 0x19,
        right.2,
    ]);
    r
}

struct Void {
    hid: HidDevice,
    host: bool,
    asked: Instant,
}

impl Driver for Void {
    fn frame(&mut self, colors: &[Color], _first: bool) -> Result<(), String> {
        if !self.host || self.asked.elapsed() >= REFRESH {
            write(&self.hid, &lighting_control(true))?;
            self.host = true;
            self.asked = Instant::now();
        }
        write(&self.hid, &set_colors(colors[0], colors[1]))
    }
}

impl Drop for Void {
    fn drop(&mut self) {
        if self.host {
            let _ = write(&self.hid, &lighting_control(false));
        }
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("corsairvoid"),
        det.name,
        DeviceKind::Other,
        vec![[0.25, 0.5], [0.75, 0.5]],
        [2, 1],
    );
    let driver = Void {
        hid,
        host: false,
        asked: Instant::now(),
    };
    Ok(Lights::boxed("corsair void", dev, None, driver))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(DETECTORS.len(), 15);
    }

    // CorsairVoidController::SetLightingControl
    #[test]
    fn lighting_control_reports() {
        assert_eq!(lighting_control(true), [0xC8, 0x01, 0x00]);
        assert_eq!(lighting_control(false), [0xC8, 0x00, 0x00]);
    }

    // CorsairVoidController::SetColors
    #[test]
    fn color_report() {
        let r = set_colors(Color(1, 2, 3), Color(4, 5, 6));
        assert_eq!(
            r,
            [
                0xCB, 0x06, 0x1C, 1, 0x16, 2, 0x17, 3, 0x1D, 4, 0x18, 5, 0x19, 6, 0, 0, 0, 0, 0, 0
            ]
        );
    }
}
