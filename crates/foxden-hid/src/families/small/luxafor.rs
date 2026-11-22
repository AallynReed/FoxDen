// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "luxafor";
const LEDS: usize = 6;
const DIRECT: u8 = 1;

pub fn direct(led: u8, c: Color) -> [u8; 9] {
    [0, DIRECT, led, c.0, c.1, c.2, 100, 0, 0]
}

struct Luxafor;

impl Proto for Luxafor {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        (0..LEDS).try_for_each(|i| io.write(&direct(i as u8 + 1, colors[i])))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Other,
        3,
        2,
        &[(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)],
    );
    boxed(api, info, FAMILY, dev, Luxafor)
}

pub static DETECTORS: &[Detector] = &[det("Luxafor Flag", 0x04D8, 0xF372, ANY, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // LuxaforController::SendPacket in direct mode, LED values 1..6
    #[test]
    fn six_leds() {
        let a: Vec<Color> = (0..6).map(|i| Color(i, 10 + i, 20 + i)).collect();
        let [first, _, _] = pushes(Luxafor, LEDS, vec![], &a, &[Color::BLACK; 6]);
        let e: Vec<Op> = (0..6u8)
            .map(|i| w(vec![0, 1, i + 1, i, 10 + i, 20 + i, 100, 0, 0]))
            .collect();
        assert_eq!(first, e);
    }
}
