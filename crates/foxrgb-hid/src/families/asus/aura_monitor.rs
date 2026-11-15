//! Ported from OpenRGB Controllers/AsusAuraUSBController/AsusAuraMonitorController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-monitor";
const VID: u16 = 0x0B05;
const XG279Q: u16 = 0x1919;
const XG32VC: u16 = 0x1968;
const PG32UQ: u16 = 0x19B9;

fn feature(reg: u8, value: u8) -> [u8; 8] {
    [0x03, 0x02, 0xA1, 0x80, reg, value, 0, 0]
}

/// BeginUpdate, UpdateLed for each LED, then ApplyChanges.
pub fn reports(pid: u16, colors: &[Color]) -> Vec<[u8; 8]> {
    let mut out = Vec::new();
    if pid == PG32UQ || pid == XG32VC {
        out.extend([feature(0x20, 0), feature(0x30, 0)]);
    }
    let offset = if pid == XG279Q { 0 } else { 16 };
    for (i, c) in colors.iter().enumerate() {
        let at = offset + i as u8 * 3;
        out.extend([feature(at, c.0), feature(at + 1, c.2), feature(at + 2, c.1)]);
    }
    out.push(feature(0xA0, 0x01));
    out
}

struct Monitor(u16);

impl Proto for Monitor {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for r in reports(self.0, colors) {
            io.feature(&r)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Strip, 3);
    boxed(api, info, FAMILY, dev, Monitor(info.pid))
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS ROG Strix XG27AQ", VID, 0x198C, ANY, 0xFFA0, 1, open),
    det("ASUS ROG Strix XG27AQM", VID, 0x19BB, ANY, 0xFFA0, 1, open),
    det("ASUS ROG Strix XG279Q", VID, XG279Q, ANY, 0xFFA0, 1, open),
    det("ASUS ROG Strix XG27W", VID, 0x1933, ANY, 0xFFA0, 1, open),
    det("ASUS ROG Strix XG32VC", VID, XG32VC, ANY, 0xFFA0, 1, open),
    det("ASUS ROG PG32UQ", VID, PG32UQ, ANY, 0xFFA0, 1, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AuraMonitor::DeviceUpdateLEDs
    #[test]
    fn frame_reports() {
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let [first, _, _] = pushes(Monitor(PG32UQ), 3, vec![], &a, &[]);
        let f = |r: u8, v: u8| Op::F(vec![3, 2, 0xA1, 0x80, r, v, 0, 0]);
        assert_eq!(first.len(), 2 + 9 + 1);
        assert_eq!(
            first[..5],
            [f(0x20, 0), f(0x30, 0), f(16, 1), f(17, 3), f(18, 2)]
        );
        assert_eq!(first[10], f(24, 8));
        assert_eq!(first[11], f(0xA0, 1));
        let [first, _, _] = pushes(Monitor(XG279Q), 3, vec![], &a, &[]);
        assert_eq!(first[..3], [f(0, 1), f(1, 3), f(2, 2)]);
        assert_eq!(first.len(), 10);
    }
}
