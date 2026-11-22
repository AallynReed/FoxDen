// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, report, rows, write};
use crate::registry::{Detector, Info};

const PER_SIDE: usize = 96;
const SIDE_DATA: usize = 436;

pub static DETECTORS: &[Detector] = &[det!(
    "Cooler Master GD160 ARGB Gaming Desk",
    0x2516,
    0x01A9,
    Some(0),
    Some(0xFF00),
    Some(1),
    open
)];

pub type Report = [u8; 65];

/// ResetDevice, which the constructor sends.
pub fn reset() -> [Report; 3] {
    [
        report(&[0x80, 0x11]),
        report(&[0x80, 0x0B, 0x01, 0x02]),
        report(&[0x80, 0x18, 0x01, 0x02]),
    ]
}

/// SetControlMode for the front (1) and back (2) sides.
pub fn control_mode(software: bool) -> [Report; 2] {
    [1u8, 2].map(|side| report(&[0x80, 0x07, 0x02, side, 0x00, software as u8]))
}

/// SendColorData in Direct mode (command 0x07, mode byte 0x01, brightness 0xFF).
pub fn direct(colors: &[Color]) -> Vec<Report> {
    let mut data = [0u8; 2 * SIDE_DATA];
    for (i, c) in colors.iter().take(2 * PER_SIDE).enumerate() {
        let at = (i / PER_SIDE) * SIDE_DATA + (i % PER_SIDE) * 3;
        data[at..at + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    let mut out = Vec::with_capacity(14);
    for side in 1..=2u8 {
        let base = (side as usize - 1) * SIDE_DATA;
        for packet in 0..7usize {
            let mut r = [0u8; 65];
            r[1] = if packet < 6 { packet as u8 } else { 0x86 };
            if packet == 0 {
                r[2..7].copy_from_slice(&[0x07, 0x02, side, 0x01, 0xFF]);
                r[7..].copy_from_slice(&data[base..base + 58]);
            } else {
                let at = base + 58 + (packet - 1) * 63;
                r[2..].copy_from_slice(&data[at..at + 63]);
            }
            out.push(r);
        }
    }
    out
}

struct Desk {
    hid: HidDevice,
}

impl Driver for Desk {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            for r in reset().iter().chain(&control_mode(true)) {
                write(&self.hid, r)?;
            }
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
        rows(&[PER_SIDE, PER_SIDE]),
        [PER_SIDE as u32, 2],
    );
    Ok(Lights::boxed(
        "cooler master gd160",
        dev,
        None,
        Desk { hid },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMGD160Controller::ResetDevice
    #[test]
    fn reset_bytes() {
        let [a, b, c] = reset();
        assert_eq!(&a[..4], &[0x00, 0x80, 0x11, 0x00]);
        assert_eq!(&b[..6], &[0x00, 0x80, 0x0B, 0x01, 0x02, 0x00]);
        assert_eq!(&c[..6], &[0x00, 0x80, 0x18, 0x01, 0x02, 0x00]);
    }

    // CMGD160Controller::SetControlMode
    #[test]
    fn control_mode_bytes() {
        let [front, back] = control_mode(true);
        assert_eq!(
            &front[..8],
            &[0x00, 0x80, 0x07, 0x02, 0x01, 0x00, 0x01, 0x00]
        );
        assert_eq!(
            &back[..8],
            &[0x00, 0x80, 0x07, 0x02, 0x02, 0x00, 0x01, 0x00]
        );
    }

    // CMGD160Controller::SendColorData(colors, 0x07, 0x01, 0xFF, true)
    #[test]
    fn direct_bytes() {
        let mut colors = vec![Color::BLACK; 2 * PER_SIDE];
        colors[0] = Color(1, 2, 3);
        colors[95] = Color(4, 5, 6);
        colors[96] = Color(7, 8, 9);
        let p = direct(&colors);
        assert_eq!(p.len(), 14);
        assert_eq!(
            &p[0][..10],
            &[0x00, 0x00, 0x07, 0x02, 0x01, 0x01, 0xFF, 1, 2, 3]
        );
        // LED 95 at side byte 285: packet 4 covers 247..310
        assert_eq!(&p[4][..2], &[0x00, 0x04]);
        assert_eq!(&p[4][2 + 285 - 247..2 + 288 - 247], &[4, 5, 6]);
        assert_eq!(p[6][1], 0x86);
        assert_eq!(
            &p[7][..10],
            &[0x00, 0x00, 0x07, 0x02, 0x02, 0x01, 0xFF, 7, 8, 9]
        );
        assert_eq!(p[13][1], 0x86);
    }
}
