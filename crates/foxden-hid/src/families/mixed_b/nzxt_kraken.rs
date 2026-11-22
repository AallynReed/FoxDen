// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, any, at, device, open_with};
use crate::registry::{Detector, Info};

/// Logo, then the eight ring LEDs.
const LEDS: usize = 9;

/// `NZXTKrakenController::UpdateEffect` on the sync channel in fixed mode, as Direct sends it:
/// the logo colour goes out with red and green swapped.
fn fixed_sync(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x02;
    b[1] = 0x4C;
    b[2] = 0x00;
    b[3] = 0x00;
    b[4] = 0x02;
    for i in 0..LEDS {
        let c = at(colors, i);
        let rgb = if i == 0 {
            [c.1, c.0, c.2]
        } else {
            [c.0, c.1, c.2]
        };
        b[5 + i * 3..8 + i * 3].copy_from_slice(&rgb);
    }
    b
}

struct Kraken;

impl Protocol for Kraken {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![Op::Write(fixed_sync(colors))]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "nzxt kraken", |_| {
        let mut leds = vec![[0.5, 0.5]];
        leds.extend((0..8).map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            [0.5 + 0.4 * a.cos(), 0.5 + 0.4 * a.sin()]
        }));
        let dev = device(
            info.device_id("nzxt-kraken"),
            d.name,
            DeviceKind::Other,
            [3, 3],
            leds,
        );
        Ok((dev, Kraken))
    })
}

pub(super) static DETECTORS: [Detector; 2] = [
    any("NZXT Kraken X2", 0x1E71, 0x170E, open),
    any("NZXT Kraken M2", 0x1E71, 0x1715, open),
];

#[cfg(test)]
mod tests {
    use super::*;

    // RGBController_NZXTKraken::DeviceUpdateLEDs -> UpdateChannel(SYNC) -> SendEffect
    #[test]
    fn frame_is_one_sync_effect() {
        let colors: Vec<Color> = (0..9)
            .map(|i| Color(10 * i, 10 * i + 1, 10 * i + 2))
            .collect();
        let ops = Kraken.frame(&colors);
        let mut want = vec![0u8; 64];
        want[..5].copy_from_slice(&[0x02, 0x4C, 0x00, 0x00, 0x02]);
        want[5..8].copy_from_slice(&[1, 0, 2]);
        for i in 1..9u8 {
            let at = 5 + i as usize * 3;
            want[at..at + 3].copy_from_slice(&[10 * i, 10 * i + 1, 10 * i + 2]);
        }
        assert_eq!(ops, vec![Op::Write(want)]);
    }
}
