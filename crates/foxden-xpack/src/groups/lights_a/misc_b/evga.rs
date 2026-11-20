//! EVGA X15/X17 mice (protocol learned from SignalRGB's X-15_Mouse.js and X-17_Mouse.js).

use super::super::common::{MOUSE_RECT, full, keyed};
use crate::hid::{Driver, Io, Proto, open_io};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x3842;
const PAGE: u16 = 0x0008;
const USAGE: u16 = 0x004B;
const MOUSE_LEN: usize = 40;
const MOUSE_CELLS: [[u8; 2]; 4] = [[1, 0], [3, 0], [2, 1], [2, 3]];

/// Mouse zone: feature report 7, `[7, 0xEA, zone, 2, (0xFF, r, g, b) x 4, 1, 1, 1, 1]`.
pub fn mouse_zone(zone: u8, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x07, 0xEA, zone, 0x02];
    for c in colors.iter().take(4) {
        p.extend([0xFF, c.0, c.1, c.2]);
    }
    p.resize(20, 0);
    p.extend([0x01; 4]);
    p
}

fn mouse_apply(zone: u8) -> [u8; 8] {
    [0x07, 0xEA, zone, 0x01, 0x01, 0x01, 0x01, 0x01]
}

struct Mouse;

impl Proto for Mouse {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for zone in [1, 2] {
            io.feature(&mouse_zone(zone, colors), MOUSE_LEN)?;
            let _ = io.get_feature(0x07, MOUSE_LEN);
        }
        for zone in [1, 2] {
            io.feature(&mouse_apply(zone), MOUSE_LEN)?;
        }
        Ok(())
    }
}

fn open_mouse(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("evga"),
        d.name,
        DeviceKind::Mouse,
        MOUSE_RECT,
        [4, 4],
        &MOUSE_CELLS,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "evga", Mouse).boxed())
}

pub static DETECTORS: &[Detector] = &[
    full("EVGA X15 Mouse", VID, 0x2415, 1, PAGE, USAGE, open_mouse),
    full("EVGA X17 Mouse", VID, 0x240D, 1, PAGE, USAGE, open_mouse),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_reports() {
        let m = mouse_zone(2, &[Color(1, 2, 3); 4]);
        assert_eq!(m[..8], [7, 0xEA, 2, 2, 0xFF, 1, 2, 3]);
        assert_eq!(m[20..24], [1, 1, 1, 1]);
        assert_eq!(mouse_apply(1), [7, 0xEA, 1, 1, 1, 1, 1, 1]);
    }
}
