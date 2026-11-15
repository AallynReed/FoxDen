//! Ported from OpenRGB Controllers/AsusAuraUSBController/AsusROGAllyController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-rog-ally";
const VID: u16 = 0x0B05;
/// Direct's default brightness.
const BRIGHTNESS: u8 = 3;

fn feature(head: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..head.len()].copy_from_slice(head);
    b
}

/// SendInitialization, from the controller constructor.
pub fn init() -> [[u8; 64]; 2] {
    let mut id = vec![0x5D];
    id.extend_from_slice(b"ASUS Tech.Inc.");
    [feature(&[0x5D, 0xB9]), feature(&id)]
}

/// UpdateLeds: the four stick LEDs.
pub fn leds(colors: &[Color]) -> [u8; 64] {
    let mut b = feature(&[0x5A, 0xD1, 0x08, 0x0C]);
    for (i, c) in colors.iter().enumerate() {
        b[4 + i * 3..7 + i * 3].copy_from_slice(&rgb(*c));
    }
    b
}

struct Ally;

impl Proto for Ally {
    /// DeviceUpdateMode(Direct) sends UpdateBrightness first.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&feature(&[0x5A, 0xBA, 0xC5, 0xC4, BRIGHTNESS]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&leds(colors))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    for f in init() {
        io.feature(&f)?;
    }
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Other,
        4,
        1,
        &[(0, 0), (1, 0), (2, 0), (3, 0)],
    );
    Ok(Box::new(Driver::new(io, dev, FAMILY, Ally)))
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS ROG Ally", VID, 0x1ABE, 2, 0xFF31, 0x0076, open),
    det("ASUS ROG Ally X", VID, 0x1B4C, 2, 0xFF31, 0x0076, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SendInitialization, then DeviceUpdateMode(Direct) and UpdateLeds
    #[test]
    fn init_then_frame() {
        let [a, b] = init();
        assert_eq!(a.to_vec(), pad(&[0x5D, 0xB9], 64));
        assert_eq!(
            b.to_vec(),
            pad(
                &[
                    0x5D, 0x41, 0x53, 0x55, 0x53, 0x20, 0x54, 0x65, 0x63, 0x68, 0x2E, 0x49, 0x6E,
                    0x63, 0x2E
                ],
                64
            )
        );
        let c = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        let [first, _, _] = pushes(Ally, 4, vec![], &c, &[]);
        assert_eq!(
            first,
            vec![
                Op::F(pad(&[0x5A, 0xBA, 0xC5, 0xC4, 3], 64)),
                Op::F(pad(
                    &[0x5A, 0xD1, 8, 0x0C, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
                    64
                )),
            ]
        );
    }
}
