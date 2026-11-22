// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-monitor";
/// The LED count reply is awaited; this bounds the wait.
const REPLY_MS: i32 = 1000;

fn packet(head: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[..head.len()].copy_from_slice(head);
    b
}

/// GetNumberOfLEDs: the count is byte 32 of the reply.
pub fn led_count(io: &mut dyn Io) -> Result<usize, String> {
    io.write(&packet(&[0xEC, 0xB0]))?;
    let mut reply = [0u8; 65];
    let n = io.read(&mut reply, REPLY_MS)?;
    Ok(if n > 0 { reply[32] as usize } else { 0 })
}

pub fn init() -> [u8; 65] {
    packet(&[0xEC, 0x35, 0, 0, 0, 0xFF, 0, 0, 0x01])
}

pub fn direct(colors: &[Color]) -> [u8; 65] {
    let mut b = packet(&[0xEC, 0x40, 0x84, 0x00, colors.len() as u8]);
    for (i, c) in colors.iter().enumerate() {
        b[5 + 3 * i..8 + 3 * i].copy_from_slice(&rgb(*c));
    }
    b
}

struct AsusMonitor;

impl Proto for AsusMonitor {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&direct(colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    // the RGBController constructor reads the LED count, then sends the init report
    let n = led_count(io.as_mut())?.min(20);
    io.write(&init())?;
    let dev = line(info, FAMILY, d, DeviceKind::Strip, n);
    Ok(Box::new(Driver::new(io, dev, FAMILY, AsusMonitor)))
}

pub static DETECTORS: &[Detector] = &[
    det(
        "Asus ROG STRIX XG27AQDMG",
        0x0B05,
        0x1BA3,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG STRIX XG27ACDNG",
        0x0B05,
        0x1BC9,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG STRIX XG27UCG",
        0x0B05,
        0x1BB4,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG SWIFT PG32UCDM",
        0x0B05,
        0x1B2B,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG SWIFT PG32UCDMR",
        0x0B05,
        0x1C9B,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG SWIFT PG32UCDP",
        0x0B05,
        0x1BCA,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG STRIX XG32UCWG",
        0x0B05,
        0x1C89,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG STRIX XG27AQWMG",
        0x0B05,
        0x1CBC,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG SWIFT PG32UCDM3",
        0x0B05,
        0x1D37,
        1,
        0xFF72,
        0x00A1,
        open,
    ),
    det(
        "Asus ROG Aura Monitor Light Bar",
        0x0B05,
        0x1AC8,
        2,
        0xFF72,
        0x00A1,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // GetNumberOfLEDs and SendInit from the constructor
    #[test]
    fn count_then_init() {
        let mut reply = vec![0u8; 65];
        reply[32] = 3;
        let mut rec = Rec::with(vec![reply]);
        assert_eq!(led_count(&mut rec).unwrap(), 3);
        assert_eq!(rec.take(), vec![w(pad(&[0xEC, 0xB0], 65)), Op::Read]);
        assert_eq!(
            init().to_vec(),
            pad(&[0xEC, 0x35, 0, 0, 0, 0xFF, 0, 0, 1], 65)
        );
    }

    // AsusMonitorController::SetDirect
    #[test]
    fn direct_report() {
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let [first, _, _] = pushes(AsusMonitor, 3, vec![], &a, &[]);
        assert_eq!(
            first,
            vec![w(pad(
                &[0xEC, 0x40, 0x84, 0, 3, 1, 2, 3, 4, 5, 6, 7, 8, 9],
                65
            ))]
        );
    }
}
