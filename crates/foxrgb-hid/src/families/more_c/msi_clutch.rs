//! Ported from OpenRGB Controllers/MSIClutchController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "msi-clutch";
const STATIC: u8 = 0x02;

/// SendPacket: eight bytes with a 0xFF-minus-sum checksum, behind report id 0.
fn packet(io: &mut dyn Io, head: &[u8]) -> Result<(), String> {
    let mut b = [0u8; 9];
    b[1..1 + head.len()].copy_from_slice(head);
    let sum = b[1..8].iter().fold(0u8, |s, &x| s.wrapping_add(x));
    b[8] = 0xFF - sum;
    io.feature(&b)?;
    io.sleep(5);
    Ok(())
}

/// SendCustomPalette's 33-byte palette with the custom colour in slot 7.
pub fn palette(c: Color) -> [u8; 33] {
    let mut b = [0u8; 33];
    b[1..22].copy_from_slice(&[
        0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0xFF, 0xFF,
        0xFF, 0xFF,
    ]);
    b[22..25].copy_from_slice(&rgb(c));
    b[32] = 0xD0;
    b
}

struct Clutch;

impl Proto for Clutch {
    // SetMode(Static) at full brightness
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        packet(io, &[0x03, 0x03, 0x0F, 0xF2, 0x40])?;
        packet(io, &[0x85, STATIC])?;
        packet(io, &[0x05, STATIC, 0x00, 0xFF])?;
        packet(io, &[0x10, 0x00, 0x18])?;
        io.write(&palette(colors[0]))?;
        io.sleep(5);
        packet(io, &[0x05, STATIC, 0x08, 0x07])
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Clutch)
}

pub static DETECTORS: &[Detector] = &[det(
    "MSI Clutch GM11 controller",
    0x0DB0,
    0x0D11,
    2,
    0xFF00,
    0xFF00,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn static_sequence() {
        let [first, _, _] = pushes(Clutch, 1, vec![], &[Color(1, 2, 3)], &[]);
        let s = Op::Sleep(5);
        let mut pal = pad(
            &[
                0, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0xFF,
                0xFF, 0xFF, 0xFF, 1, 2, 3,
            ],
            33,
        );
        pal[32] = 0xD0;
        assert_eq!(
            first,
            vec![
                Op::F(vec![0, 3, 3, 0x0F, 0xF2, 0x40, 0, 0, 0xB8]),
                s.clone(),
                Op::F(vec![0, 0x85, 2, 0, 0, 0, 0, 0, 0x78]),
                s.clone(),
                Op::F(vec![0, 5, 2, 0, 0xFF, 0, 0, 0, 0xF9]),
                s.clone(),
                Op::F(vec![0, 0x10, 0, 0x18, 0, 0, 0, 0, 0xD7]),
                s.clone(),
                w(pal),
                s.clone(),
                Op::F(vec![0, 5, 2, 8, 7, 0, 0, 0, 0xE9]),
                s,
            ]
        );
    }
}
