// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, rows};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

static DETECTORS: [Detector; 1] = [Detector {
    name: "MSI 3-Zone Laptop",
    vid: 0x1770,
    pid: 0xFF00,
    interface: None,
    usage_page: None,
    usage: None,
    open,
}];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

/// MSI3ZoneController::SetLEDs: left, middle, right, aux, then regions 5 to 7 repeat the aux colour.
fn reports(colors: &[Color]) -> Vec<Op> {
    let mut buf = [1u8, 2, 64, 0, 0, 0, 0, 236];
    (1..=7u8)
        .map(|region| {
            buf[3] = region;
            if let Some(c) = colors.get(region as usize - 1).filter(|_| region <= 4) {
                buf[4..7].copy_from_slice(&[c.0, c.1, c.2]);
            }
            Op::Feature(buf.to_vec())
        })
        .collect()
}

struct ThreeZone;

impl Proto for ThreeZone {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        reports(colors)
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("msi-3zone"),
        det.name,
        DeviceKind::Keyboard,
        rows(&[3, 1]),
    );
    Ok(Driver::boxed(hid, "msi 3-zone", dev, ThreeZone))
}

#[cfg(test)]
mod tests {
    use super::*;

    // MSI3ZoneController::SetLEDs.
    #[test]
    fn seven_reports() {
        let ops = reports(&[
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ]);
        let expect: Vec<Op> = [
            [1, 2, 64, 1, 1, 2, 3, 236],
            [1, 2, 64, 2, 4, 5, 6, 236],
            [1, 2, 64, 3, 7, 8, 9, 236],
            [1, 2, 64, 4, 10, 11, 12, 236],
            [1, 2, 64, 5, 10, 11, 12, 236],
            [1, 2, 64, 6, 10, 11, 12, 236],
            [1, 2, 64, 7, 10, 11, 12, 236],
        ]
        .iter()
        .map(|b| Op::Feature(b.to_vec()))
        .collect();
        assert_eq!(ops, expect);
    }
}
