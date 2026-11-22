// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, exchange, report, write};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 250;

pub static DETECTORS: &[Detector] = &[det!(
    "Cooler Master MM712",
    0x2516,
    0x0169,
    Some(3),
    Some(0xFF0A),
    Some(2),
    open
)];

/// SendInitPacket
pub fn init() -> [u8; 65] {
    report(&[0x44, 0x81, 0x02])
}

/// SetDirectMode
pub fn direct_mode(on: bool) -> [u8; 65] {
    report(&[0x5A, 0x81, 0x01 + on as u8])
}

/// SetLedsDirect: OpenRGB never records direct mode as current, so every frame turns it on first.
pub fn direct(c: Color) -> [[u8; 65]; 2] {
    [
        direct_mode(true),
        report(&[0x5A, 0x81, 0x03, c.0, c.1, c.2]),
    ]
}

struct Mm712 {
    hid: HidDevice,
}

impl Driver for Mm712 {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            exchange(&self.hid, &init(), TIMEOUT)?;
            write(&self.hid, &direct_mode(true))?;
        }
        for r in direct(colors[0]) {
            write(&self.hid, &r)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Mouse,
        vec![[0.5, 0.5]],
        [1, 1],
    );
    Ok(Lights::boxed(
        "cooler master mm712",
        dev,
        None,
        Mm712 { hid },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMMM712Controller::SendInitPacket
    #[test]
    fn init_bytes() {
        assert_eq!(&init()[..5], &[0x00, 0x44, 0x81, 0x02, 0x00]);
    }

    // CMMM712Controller::SetDirectMode
    #[test]
    fn direct_mode_bytes() {
        assert_eq!(direct_mode(true), report(&[0x5A, 0x81, 0x02]));
        assert_eq!(direct_mode(false), report(&[0x5A, 0x81, 0x01]));
    }

    // CMMM712Controller::SetLedsDirect
    #[test]
    fn direct_bytes() {
        let [on, color] = direct(Color(7, 8, 9));
        assert_eq!(on, direct_mode(true));
        assert_eq!(&color[..8], &[0x00, 0x5A, 0x81, 0x03, 7, 8, 9, 0]);
    }
}
