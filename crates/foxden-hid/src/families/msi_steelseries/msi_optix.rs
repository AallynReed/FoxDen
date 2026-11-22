// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const LEDS: usize = 12;
const BRIGHTNESS_MAX: u8 = 0x64;

static DETECTORS: [Detector; 1] = [Detector {
    name: "MSI Optix controller",
    vid: 0x1462,
    pid: 0x3FA4,
    interface: Some(0),
    usage_page: Some(0xFF00),
    usage: Some(1),
    open,
}];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

/// MSIOptixController::SetDirect.
fn direct_report(colors: &[Color], brightness: u8) -> [u8; 168] {
    let mut buf = [0u8; 168];
    buf[0] = 0x72;
    for base in [0usize, 0x0B] {
        buf[base + 1..base + 12].copy_from_slice(&[
            0x01, 0xFF, 0xFF, 0xFF, 0x00, brightness, 0x00, 0xFF, 0x00, 0x80, 0x00,
        ]);
    }
    let start = 0x0B + 0x0B + 24 * 3 + 1;
    for (i, c) in colors.iter().take(LEDS).enumerate() {
        buf[start + i * 3..start + i * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    buf
}

struct Optix;

impl Proto for Optix {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![Op::Feature(direct_report(colors, BRIGHTNESS_MAX).to_vec())]
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("msi-optix"),
        det.name,
        DeviceKind::Other,
        line(LEDS),
    );
    Ok(Driver::boxed(hid, "msi optix", dev, Optix))
}

#[cfg(test)]
mod tests {
    use super::*;

    // MSIOptixController::SetDirect with Direct's brightness 0x64.
    #[test]
    fn direct_bytes() {
        let colors: Vec<Color> = (0..12).map(|i| Color(i, i + 100, i + 200)).collect();
        let p = direct_report(&colors, 0x64);
        let block = [
            0x01, 0xFF, 0xFF, 0xFF, 0x00, 0x64, 0x00, 0xFF, 0x00, 0x80, 0x00,
        ];
        assert_eq!(p[0], 0x72);
        assert_eq!(p[1..12], block);
        assert_eq!(p[12..23], block);
        assert!(p[23..95].iter().all(|&b| b == 0));
        assert_eq!(p[95..98], [0, 100, 200]);
        assert_eq!(p[95 + 33..95 + 36], [11, 111, 211]);
        assert!(p[131..].iter().all(|&b| b == 0));
    }
}
