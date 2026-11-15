//! Ported from OpenRGB Controllers/AsusAuraUSBController/AsusAuraHeadsetStandController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-headset-stand";
const VID: u16 = 0x0B05;
const LEDS: usize = 18;

/// UpdateLeds: 15 colours, then the last three (underglow 15-16 and the logo).
pub fn packets(colors: &[Color]) -> [[u8; 65]; 2] {
    let mut a = report(&[0x00, 0xC0, 0x81, 0x00, 0x00]);
    for (i, c) in colors[..15].iter().enumerate() {
        a[6 + i * 4..9 + i * 4].copy_from_slice(&rgb(*c));
    }
    let mut b = report(&[0x00, 0xC0, 0x81, 0x01, 0x00]);
    for (i, c) in colors[15..18].iter().enumerate() {
        b[6 + i * 4..9 + i * 4].copy_from_slice(&rgb(*c));
    }
    [a, b]
}

struct Stand;

impl Proto for Stand {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in packets(colors) {
            io.write(&p)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut at: Vec<(u32, u32)> = (0..LEDS as u32 - 1).map(|c| (c, 1)).collect();
    at.push((8, 0));
    let dev = cells(info, FAMILY, d, DeviceKind::Other, 17, 2, &at);
    boxed(api, info, FAMILY, dev, Stand)
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS ROG Throne", VID, 0x18D9, 0, ANY, ANY, open),
    det("ASUS ROG Throne QI", VID, 0x18C5, 0, ANY, ANY, open),
    det("ASUS ROG Throne QI GUNDAM", VID, 0x1994, 0, ANY, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AuraHeadsetStand::DeviceUpdateLEDs
    #[test]
    fn frame_packets() {
        let a: Vec<Color> = (0..LEDS as u8).map(|i| Color(i, i + 1, i + 2)).collect();
        let [first, _, _] = pushes(Stand, LEDS, vec![], &a, &[]);
        let mut p0 = pad(&[0, 0xC0, 0x81, 0, 0], 65);
        for i in 0..15u8 {
            let at = 5 + i as usize * 4;
            p0[at..at + 4].copy_from_slice(&[0, i, i + 1, i + 2]);
        }
        let p1 = pad(
            &[
                0, 0xC0, 0x81, 1, 0, 0, 15, 16, 17, 0, 16, 17, 18, 0, 17, 18, 19,
            ],
            65,
        );
        assert_eq!(first, vec![w(p0), w(p1)]);
    }
}
