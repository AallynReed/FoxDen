//! Ported from OpenRGB Controllers/HPOmen30LController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "hp-omen-30l";
const ZONES: usize = 7;
const DIRECT: u8 = 0x04;
const MAX_BRIGHTNESS: u8 = 0x64;

/// SendZoneUpdate in direct mode; zone values run 1..=7.
pub fn zone_update(zone: usize, c: Color, brightness: u8) -> [u8; 58] {
    let mut b = [0u8; 58];
    let index = zone;
    b[0x02] = 0x12;
    b[0x03] = DIRECT;
    b[0x04] = 0x01;
    b[0x05] = 0x01;
    b[0x08 + index * 4..0x0C + index * 4].copy_from_slice(&[MAX_BRIGHTNESS, c.0, c.1, c.2]);
    b[0x30] = brightness;
    b[0x31] = DIRECT;
    b[0x36] = zone as u8 + 1;
    b[0x37] = 0x01;
    b
}

struct Omen;

impl Proto for Omen {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        (0..ZONES).try_for_each(|z| io.write(&zone_update(z, colors[z], 100)))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Other,
        3,
        4,
        &[(1, 0), (1, 1), (0, 1), (2, 2), (0, 3), (0, 2), (0, 0)],
    );
    boxed(api, info, FAMILY, dev, Omen)
}

pub static DETECTORS: &[Detector] = &[det("HP Omen 30L", 0x103C, 0x84FD, ANY, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // HPOmen30LController::SendZoneUpdate in direct mode for each zone
    #[test]
    fn seven_zone_reports() {
        let a: Vec<Color> = (0..7).map(|i| Color(i, 2 * i, 3 * i)).collect();
        let [first, _, _] = pushes(Omen, ZONES, vec![], &a, &[]);
        assert_eq!(first.len(), 7);
        let mut z2 = pad(&[0, 0, 0x12, 4, 1, 1], 58);
        z2[0x08 + 8..0x0C + 8].copy_from_slice(&[0x64, 2, 4, 6]);
        z2[0x30] = 100;
        z2[0x31] = 4;
        z2[0x36] = 3;
        z2[0x37] = 1;
        assert_eq!(first[2], w(z2));
    }
}
