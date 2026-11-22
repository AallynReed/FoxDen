// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, exchange, line, report};
use crate::registry::{Detector, Info};

const VID: u16 = 0x2516;
const I1: Option<i32> = Some(1);
const PAGE: Option<u16> = Some(0xFF00);
const USAGE: Option<u16> = Some(1);
const TIMEOUT: i32 = 250;
const MM530: u16 = 0x0065;
const MM531: u16 = 0x0097;
const CUSTOM: u8 = 176;
const BRIGHTNESS_MAX: u8 = 0xFF;

pub static DETECTORS: &[Detector] = &[
    det!("Cooler Master MM530", VID, MM530, I1, PAGE, USAGE, open),
    det!("Cooler Master MM720", VID, 0x0141, I1, PAGE, USAGE, open),
    det!("Cooler Master MM730", VID, 0x0165, I1, PAGE, USAGE, open),
];

fn five_series(pid: u16) -> bool {
    pid == MM530 || pid == MM531
}

fn command(pid: u16) -> u8 {
    if five_series(pid) { 0x2C } else { 0x2B }
}

/// SendInitPacket
pub fn init() -> [u8; 65] {
    report(&[0x41, 0x80])
}

/// SendUpdate(Custom, speed 0, black, full brightness): zones packet, mode packet, apply.
pub fn custom_mode(pid: u16) -> [[u8; 65]; 3] {
    let mut update = report(&[
        0x51,
        command(pid),
        0x00,
        0x00,
        CUSTOM,
        0x00,
        0x20,
        0xFF,
        0xFF,
    ]);
    update[10] = BRIGHTNESS_MAX;
    update[17..].fill(0xFF);
    [
        report(&[0x51, 0x30]),
        update,
        report(&[0x51, 0x28, 0x00, 0x00, CUSTOM]),
    ]
}

/// SetLedsDirect: wheel, (buttons on the MM530) and logo.
pub fn direct(pid: u16, colors: &[Color]) -> [u8; 65] {
    let (wheel, buttons, logo) = if five_series(pid) {
        (colors[0], colors[1], colors[2])
    } else {
        (colors[0], Color::BLACK, colors[1])
    };
    let (button_at, wheel_at) = if five_series(pid) { (5, 11) } else { (11, 5) };
    let mut r = report(&[0x51, 0xA8, 0x00, 0x00]);
    r[button_at..button_at + 3].copy_from_slice(&[buttons.0, buttons.1, buttons.2]);
    r[8..11].copy_from_slice(&[logo.0, logo.1, logo.2]);
    r[wheel_at..wheel_at + 3].copy_from_slice(&[wheel.0, wheel.1, wheel.2]);
    r
}

struct Mouse {
    hid: HidDevice,
    pid: u16,
}

impl Driver for Mouse {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            exchange(&self.hid, &init(), TIMEOUT)?;
            for r in custom_mode(self.pid) {
                exchange(&self.hid, &r, TIMEOUT)?;
            }
        }
        exchange(&self.hid, &direct(self.pid, colors), TIMEOUT).map(|_| ())
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let n = if five_series(info.pid) { 3 } else { 2 };
    let dev = device(
        info.device_id("coolermaster"),
        det.name,
        DeviceKind::Mouse,
        line(n).into_iter().map(|[x, y]| [y, x]).collect(),
        [1, n as u32],
    );
    let driver = Mouse { hid, pid: info.pid };
    Ok(Lights::boxed("cooler master mouse", dev, None, driver))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMMMController::SendInitPacket
    #[test]
    fn init_bytes() {
        assert_eq!(&init()[..3], &[0x00, 0x41, 0x80]);
    }

    // CMMMController::SendUpdate(CM_MM_MODE_CUSTOM, ...) via SendUsingZonesPacket and SendApplyPacket
    #[test]
    fn custom_mode_bytes() {
        let [zones, update, apply] = custom_mode(0x0141);
        assert_eq!(zones, report(&[0x51, 0x30]));
        assert_eq!(
            &update[..17],
            &[
                0x00, 0x51, 0x2B, 0, 0, 0xB0, 0x00, 0x20, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0
            ]
        );
        assert!(update[17..].iter().all(|&b| b == 0xFF));
        assert_eq!(apply, report(&[0x51, 0x28, 0, 0, 0xB0]));
        assert_eq!(custom_mode(MM530)[1][2], 0x2C);
    }

    // CMMMController::SetLedsDirect for the MM7xx byte layout
    #[test]
    fn direct_mm720() {
        let r = direct(0x0141, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(
            &r[..14],
            &[0x00, 0x51, 0xA8, 0x00, 0x00, 1, 2, 3, 4, 5, 6, 0, 0, 0]
        );
    }

    // CMMMController::SetLedsDirect for the MM530 byte layout
    #[test]
    fn direct_mm530() {
        let r = direct(MM530, &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)]);
        assert_eq!(
            &r[..14],
            &[0x00, 0x51, 0xA8, 0x00, 0x00, 4, 5, 6, 7, 8, 9, 1, 2, 3]
        );
    }
}
