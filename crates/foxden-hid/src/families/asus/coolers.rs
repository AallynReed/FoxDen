//! Ported from OpenRGB Controllers/AsusAuraUSBController/AsusAuraRyuoAIOController and AsusROGStrixLCController (GPL-2.0-or-later).

use super::aura_usb::base;
use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-cooler";
const VID: u16 = 0x0B05;
const MODE_DIRECT: u8 = 0xFF;

/// Ryuo SendUpdate for Direct: speed and direction stay at the mode defaults of 0.
pub fn ryuo_mode() -> [u8; 65] {
    report(&[0xEC, 0x3B, 0x00, 0x22, MODE_DIRECT])
}

/// Ryuo SetLedsDirect.
pub fn ryuo_direct(colors: &[Color]) -> [u8; 65] {
    let mut b = report(&[0xEC, 0x40, 0x80, 0x00, colors.len() as u8]);
    for (i, c) in colors.iter().enumerate() {
        b[5 + i * 3..8 + i * 3].copy_from_slice(&rgb(*c));
    }
    b
}

/// Strix LC SendUpdate for Direct.
pub fn lc_mode() -> [u8; 65] {
    report(&[0xEC, 0x3B, 0x00, MODE_DIRECT])
}

/// Strix LC SetLedsDirect, which ends with a fixed trailer.
pub fn lc_direct(colors: &[Color]) -> [u8; 65] {
    let mut b = report(&[0xEC, 0x40, 0x00, 0xFF, colors.len() as u8]);
    for (i, c) in colors.iter().enumerate() {
        b[5 + i * 3..8 + i * 3].copy_from_slice(&rgb(*c));
    }
    let end = 5 + colors.len() * 3;
    b[end..end + 3].copy_from_slice(&[0x77, 0x10, 0xF3]);
    b
}

struct Cooler {
    lc: bool,
}

impl Proto for Cooler {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&if self.lc { lc_mode() } else { ryuo_mode() })
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&if self.lc {
            lc_direct(colors)
        } else {
            ryuo_direct(colors)
        })
    }
}

fn build(
    api: &HidApi,
    info: &Info,
    d: &Detector,
    lc: bool,
    n: usize,
) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    base(io.as_mut())?;
    let dev = ring(info, FAMILY, d, n);
    Ok(Box::new(Driver::new(io, dev, FAMILY, Cooler { lc })))
}

fn ryuo(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let n = if d.name.contains("Ryujin") { 5 } else { 12 };
    build(api, info, d, false, n)
}

fn strix_lc(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    build(api, info, d, true, 4)
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS ROG Ryuo AIO", VID, 0x1887, ANY, 0xFF72, 0x00A1, ryuo),
    det(
        "ASUS ROG Ryujin AIO",
        VID,
        0x18AE,
        ANY,
        0xFF72,
        0x00A1,
        ryuo,
    ),
    det("ASUS ROG Strix LC", VID, 0x879E, ANY, 0x00FF, 1, strix_lc),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AsusAuraRyuoAIO: DeviceUpdateMode(Direct), then SetLedsDirect
    #[test]
    fn ryuo_packets() {
        let a: Vec<Color> = (0..12u8).map(|i| Color(i, 0, 0xFF)).collect();
        let [first, _, _] = pushes(Cooler { lc: false }, 12, vec![], &a, &[]);
        let mut p = pad(&[0xEC, 0x40, 0x80, 0, 12], 65);
        for i in 0..12 {
            p[5 + i * 3..8 + i * 3].copy_from_slice(&[i as u8, 0, 0xFF]);
        }
        assert_eq!(first, vec![w(pad(&[0xEC, 0x3B, 0, 0x22, 0xFF], 65)), w(p)]);
    }

    // RGBController_AsusROGStrixLC: DeviceUpdateMode(Direct), then SetLedsDirect
    #[test]
    fn strix_lc_packets() {
        let a = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        let [first, _, _] = pushes(Cooler { lc: true }, 4, vec![], &a, &[]);
        let p = pad(
            &[
                0xEC, 0x40, 0, 0xFF, 4, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0x77, 0x10, 0xF3,
            ],
            65,
        );
        assert_eq!(first, vec![w(pad(&[0xEC, 0x3B, 0, 0xFF], 65)), w(p)]);
    }
}
