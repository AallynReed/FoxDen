// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, line, report, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x2516;
const I0: Option<i32> = Some(0);
const PAGE: Option<u16> = Some(0xFF00);
const USAGE: Option<u16> = Some(1);
const LEDS: usize = 47;
const DATA: usize = 436;

pub static DETECTORS: &[Detector] = &[
    det!(
        "Cooler Master GM27-FQS ARGB Monitor",
        VID,
        0x01BB,
        I0,
        PAGE,
        USAGE,
        open
    ),
    det!(
        "Cooler Master GM34-CWQ ARGB Monitor",
        VID,
        0x01BF,
        I0,
        PAGE,
        USAGE,
        open
    ),
];

pub type Report = [u8; 65];

/// SetSoftwareModeEnabled
pub fn software_mode(on: bool) -> Report {
    report(&[0x80, 0x07, 0x02, 0x02, 0x00, on as u8])
}

/// SendDirect: 436 colour bytes over seven packets, the last numbered 0x86.
pub fn direct(colors: &[Color]) -> [Report; 7] {
    let mut data = [0u8; DATA];
    for (i, c) in colors.iter().take(DATA / 3).enumerate() {
        data[i * 3..i * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    let mut out = [[0u8; 65]; 7];
    out[0] = report(&[0x00, 0x07, 0x02, 0x02, 0x01, 0x80]);
    out[0][7..].copy_from_slice(&data[..58]);
    for (i, r) in out.iter_mut().enumerate().skip(1) {
        r[1] = if i < 6 { i as u8 } else { 0x86 };
        let at = 58 + (i - 1) * 63;
        r[2..].copy_from_slice(&data[at..at + 63]);
    }
    out
}

struct Monitor {
    hid: HidDevice,
}

impl Driver for Monitor {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            write(&self.hid, &software_mode(true))?;
        }
        for r in direct(colors) {
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
        DeviceKind::Strip,
        line(LEDS),
        [LEDS as u32, 1],
    );
    Ok(Lights::boxed(
        "cooler master monitor",
        dev,
        None,
        Monitor { hid },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMMonitorController::SetSoftwareModeEnabled
    #[test]
    fn software_mode_bytes() {
        assert_eq!(
            &software_mode(true)[..8],
            &[0x00, 0x80, 0x07, 0x02, 0x02, 0x00, 0x01, 0x00]
        );
        assert_eq!(software_mode(false)[6], 0x00);
    }

    // CMMonitorController::SendDirect
    #[test]
    fn direct_bytes() {
        let colors: Vec<Color> = (0..LEDS as u8).map(|i| Color(i, i, i)).collect();
        let p = direct(&colors);
        assert_eq!(
            &p[0][..10],
            &[0x00, 0x00, 0x07, 0x02, 0x02, 0x01, 0x80, 0, 0, 0]
        );
        assert_eq!(&p[0][10..13], &[1, 1, 1]);
        // LED 19 starts at data byte 57, the last byte of packet 0
        assert_eq!(p[0][64], 19);
        assert_eq!(&p[1][..4], &[0x00, 0x01, 19, 19]);
        assert_eq!(p[5][1], 0x05);
        assert_eq!(p[6][1], 0x86);
        // LED 46 ends at data byte 140, packet 2 byte 2 + 140 - 121
        assert_eq!(p[2][2 + 140 - 121], 46);
        assert!(p[2][2 + 141 - 121..].iter().all(|&b| b == 0));
        assert!(p[6][2..].iter().all(|&b| b == 0));
    }
}
