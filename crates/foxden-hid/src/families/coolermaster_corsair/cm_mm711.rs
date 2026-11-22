// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, exchange, report};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 250;

pub static DETECTORS: &[Detector] = &[det!(
    "Cooler Master MM711",
    0x2516,
    0x0101,
    Some(1),
    Some(0xFF00),
    Some(1),
    open
)];

/// SendInitPacket
pub fn init() -> [u8; 65] {
    report(&[0x41, 0x80])
}

/// SetLedsDirect: scroll wheel then logo.
pub fn direct(wheel: Color, logo: Color) -> [u8; 65] {
    report(&[
        0x51, 0xA8, 0x00, 0x00, wheel.0, wheel.1, wheel.2, logo.0, logo.1, logo.2,
    ])
}

struct Mm711 {
    hid: HidDevice,
}

impl Driver for Mm711 {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            exchange(&self.hid, &init(), TIMEOUT)?;
        }
        exchange(&self.hid, &direct(colors[0], colors[1]), TIMEOUT).map(|_| ())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Mouse,
        vec![[0.5, 0.25], [0.5, 0.75]],
        [1, 2],
    );
    Ok(Lights::boxed(
        "cooler master mm711",
        dev,
        None,
        Mm711 { hid },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMMM711Controller::SendInitPacket
    #[test]
    fn init_bytes() {
        assert_eq!(init(), report(&[0x41, 0x80]));
    }

    // CMMM711Controller::SetLedsDirect
    #[test]
    fn direct_bytes() {
        let r = direct(Color(1, 2, 3), Color(4, 5, 6));
        assert_eq!(&r[..12], &[0x00, 0x51, 0xA8, 0, 0, 1, 2, 3, 4, 5, 6, 0]);
        assert!(r[11..].iter().all(|&b| b == 0));
    }
}
