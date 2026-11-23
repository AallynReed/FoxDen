// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

const PIPE_IDLE_MS: u64 = 5;
const LEGACY_LEDS: usize = 9;
const RIGHT_TO_LEFT: u8 = 0;
const LEFT_TO_RIGHT: u8 = 1;
const CIRCLE: u8 = 3;
const CTYPE: u8 = 4;

static DETECTORS: [Detector; 1] = [Detector {
    name: "MSI Monitor (Gaming Controller)",
    vid: 0x1462,
    pid: 0x3FA4,
    interface: Some(0),
    usage_page: Some(0x01),
    usage: Some(0),
    open,
}];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

/// (version, LEDs, fill direction) of the 0x72 layouts.
const LAYOUTS_72: &[(u8, usize, u8)] = &[
    (34, 9, LEFT_TO_RIGHT),
    (5, 12, RIGHT_TO_LEFT),
    (8, 24, CIRCLE),
    (10, 9, LEFT_TO_RIGHT),
    (12, 9, LEFT_TO_RIGHT),
    (14, 14, LEFT_TO_RIGHT),
    (17, 8, RIGHT_TO_LEFT),
    (20, 12, LEFT_TO_RIGHT),
    (21, 18, LEFT_TO_RIGHT),
    (22, 24, LEFT_TO_RIGHT),
    (24, 12, LEFT_TO_RIGHT),
    (25, 24, LEFT_TO_RIGHT),
    (28, 9, LEFT_TO_RIGHT),
    (29, 24, RIGHT_TO_LEFT),
    (30, 21, RIGHT_TO_LEFT),
    (32, 7, LEFT_TO_RIGHT),
    (33, 7, LEFT_TO_RIGHT),
    (36, 9, LEFT_TO_RIGHT),
    (37, 22, LEFT_TO_RIGHT),
    (38, 20, LEFT_TO_RIGHT),
    (39, 8, RIGHT_TO_LEFT),
    (40, 12, LEFT_TO_RIGHT),
    (41, 14, RIGHT_TO_LEFT),
    (42, 22, CTYPE),
    (43, 21, LEFT_TO_RIGHT),
    (44, 9, LEFT_TO_RIGHT),
    (46, 9, LEFT_TO_RIGHT),
];

fn layout_72(version: u8) -> Option<(usize, bool)> {
    LAYOUTS_72
        .iter()
        .find(|l| l.0 == version)
        .map(|l| (l.1, l.2 == RIGHT_TO_LEFT))
}

/// MSIMonitorController::Set with Direct (static, last byte 0).
fn legacy_report(colors: &[Color]) -> [u8; 78] {
    let mut d = [0u8; 78];
    d[..12].copy_from_slice(&[0x71, 0x01, 0, 0, 0, 0x01, 0x64, 0, 0, 0, 0, 0]);
    d[12..23].copy_from_slice(&[0x01, 0, 0, 0, 0x01, 0x64, 0, 0, 0, 0, 0]);
    for i in 0..9 {
        d[23 + i * 3] = 0xFF;
    }
    for (i, c) in colors.iter().take(LEGACY_LEDS).enumerate() {
        d[50 + i * 3..53 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    d[77] = 0x00;
    d
}

/// MSIMonitorController::SetMode72 as ApplyMode(false) calls it for Direct.
fn report_72(colors: &[Color], reverse: bool) -> [u8; 168] {
    let mut d = [0u8; 168];
    d[0] = 0x72;
    let c1 = colors.first().copied().unwrap_or(Color::BLACK);
    for base in [1usize, 12] {
        d[base..base + 11].copy_from_slice(&[0x01, c1.0, c1.1, c1.2, 0, 100, 0, 0, 0, 0x80, 0x00]);
    }
    let mut leds = colors.to_vec();
    if reverse {
        leds.reverse();
    }
    for (slot, c) in leds.iter().take(24).enumerate() {
        d[95 + slot * 3..98 + slot * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    d[167] = 0x00;
    d
}

/// GetLayoutVersion: drain, write [0x01][0xB0], read the reply, version at byte 3 when byte 1 is 0x5A.
fn layout_version(hid: &HidDevice) -> u8 {
    let mut reply = [0u8; 64];
    for _ in 0..64 {
        if !matches!(hid.read_timeout(&mut reply, 0), Ok(n) if n > 0) {
            break;
        }
    }
    if hid.write(&version_request()).is_err() {
        return 0;
    }
    reply = [0u8; 64];
    match hid.read_timeout(&mut reply, 500) {
        Ok(n) if n > 0 && reply[1] == 0x5A => reply[3],
        _ => 0,
    }
}

fn version_request() -> [u8; 64] {
    let mut r = [0u8; 64];
    r[0] = 0x01;
    r[1] = 0xB0;
    r
}

enum Monitor {
    Legacy,
    V72 { reverse: bool },
}

impl Proto for Monitor {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let report = match self {
            Monitor::Legacy => legacy_report(colors).to_vec(),
            Monitor::V72 { reverse } => report_72(colors, *reverse).to_vec(),
        };
        vec![Op::Feature(report), Op::Sleep(PIPE_IDLE_MS)]
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let (proto, leds) = match layout_72(layout_version(&hid)) {
        Some((leds, reverse)) => (Monitor::V72 { reverse }, leds),
        None => (Monitor::Legacy, LEGACY_LEDS),
    };
    let dev = device(
        info.device_id("msi-monitor"),
        det.name,
        DeviceKind::Other,
        line(leds),
    );
    Ok(Driver::boxed(hid, "msi monitor", dev, proto))
}

#[cfg(test)]
mod tests {
    use super::*;

    // MSIMonitorController::Set(STATIC, colors, 0x00) from RGBController_MSIMonitor Direct.
    #[test]
    fn legacy_bytes() {
        let colors: Vec<Color> = (1..=9).map(|i| Color(i, 0, i)).collect();
        let d = legacy_report(&colors);
        assert_eq!(
            d[..23],
            [
                0x71, 1, 0, 0, 0, 1, 0x64, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0x64, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(d[23..26], [0xFF, 0, 0]);
        assert_eq!(d[47..50], [0xFF, 0, 0]);
        assert_eq!(d[50..53], [1, 0, 1]);
        assert_eq!(d[74..77], [9, 0, 9]);
        assert_eq!(d[77], 0);
    }

    // SetMode72 for Direct on a right-to-left layout: Colour1 is LED 0, the array is mirrored.
    #[test]
    fn v72_bytes_mirrored() {
        assert_eq!(layout_72(17), Some((8, true)));
        let colors: Vec<Color> = (1..=8).map(|i| Color(i, i, 0)).collect();
        let d = report_72(&colors, true);
        let block = [0x01, 1, 1, 0, 0, 100, 0, 0, 0, 0x80, 0];
        assert_eq!(d[0], 0x72);
        assert_eq!(d[1..12], block);
        assert_eq!(d[12..23], block);
        assert!(d[23..95].iter().all(|&b| b == 0));
        assert_eq!(d[95..98], [8, 8, 0]);
        assert_eq!(d[95 + 21..95 + 24], [1, 1, 0]);
        assert!(d[95 + 24..].iter().all(|&b| b == 0));
    }

    // GetLayoutVersion request bytes, and an unknown version keeping the 0x71 path.
    #[test]
    fn version_query() {
        let r = version_request();
        assert_eq!(r[..2], [0x01, 0xB0]);
        assert!(r[2..].iter().all(|&b| b == 0));
        assert_eq!(layout_72(11), None);
        assert_eq!(layout_72(34), Some((9, false)));
    }
}
