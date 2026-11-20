//! Ported from OpenRGB Controllers/NvidiaESAController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "nvidia-esa";
const ZONES: usize = 5;

fn level(v: u8) -> u8 {
    (15.0f32 - 15.0 * v as f32 / 255.0) as u8
}

pub fn zone_color(zone: usize, c: Color) -> [u8; 4] {
    [0x42 + zone as u8, level(c.0), level(c.1), level(c.2)]
}

struct NvidiaEsa;

impl Proto for NvidiaEsa {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        colors
            .iter()
            .enumerate()
            .try_for_each(|(z, &c)| io.write(&zone_color(z, c)))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, ZONES);
    boxed(api, info, FAMILY, dev, NvidiaEsa)
}

pub static DETECTORS: &[Detector] = &[det(
    "Nvidia ESA - Dell XPS 730x",
    0x0955,
    0x000A,
    ANY,
    0xFFDE,
    0x02,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // NvidiaESAController::SetZoneColor for each of the five zones
    #[test]
    fn zone_reports() {
        let a = [Color(255, 0, 128); ZONES];
        let mut b = a;
        b[4] = Color(17, 18, 254);
        let [first, _, next] = pushes(NvidiaEsa, ZONES, vec![], &a, &b);
        let e: Vec<Op> = (0..5).map(|z| w(vec![0x42 + z, 0, 15, 7])).collect();
        assert_eq!(first, e);
        assert_eq!(next[4], w(vec![0x46, 14, 13, 0]));
    }
}
