// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-mouse-gen1";
const VID: u16 = 0x0B05;
const EVOLVE_DIRECT: u8 = 0x01;
const EVOLVE_BRIGHTNESS: u8 = 255;

/// SendUpdate: a 9-byte feature report, its answer, then 15 ms.
fn send_update(io: &mut dyn Io, key: u8, value: u8) -> Result<(), String> {
    io.feature(&[0x0C, 0xC4, 0x0F, 0x00, key, value, 0, 0, 0])?;
    let mut buf = [0u8; 9];
    buf[0] = 0x0C;
    io.get_feature(&mut buf)?;
    io.sleep(15);
    Ok(())
}

/// SendDirectSpatha: side, scroll wheel and logo.
pub fn spatha_direct(colors: &[Color]) -> [u8; 33] {
    let mut b = [0u8; 33];
    b[..4].copy_from_slice(&[0x10, 0x00, 0xF0, 0x00]);
    for (i, c) in colors[..3].iter().enumerate() {
        b[4 + i * 3..7 + i * 3].copy_from_slice(&rgb(*c));
    }
    b
}

struct Spatha;

impl Proto for Spatha {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&spatha_direct(colors))
    }
}

/// The Strix Evolve's Direct is a stored mode written one register at a time.
struct Evolve;

impl Proto for Evolve {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        send_update(io, 0x19, EVOLVE_DIRECT)?;
        send_update(io, 0x1A, EVOLVE_BRIGHTNESS)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let c = colors[0];
        send_update(io, 0x1C, c.0)?;
        send_update(io, 0x1D, c.1)?;
        send_update(io, 0x1E, c.2)
    }
}

fn spatha(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 3);
    boxed(api, info, FAMILY, dev, Spatha)
}

fn evolve(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Mouse, 1);
    boxed(api, info, FAMILY, dev, Evolve)
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS ROG Strix Evolve", VID, 0x185B, 1, 0x0008, ANY, evolve),
    det("ASUS ROG Spatha Wired", VID, 0x181C, 1, 0x0008, ANY, spatha),
    det(
        "ASUS ROG Spatha Wireless",
        VID,
        0x1824,
        1,
        0x0008,
        ANY,
        spatha,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // RGBController_AsusROGSpatha::DeviceUpdateLEDs in Direct
    #[test]
    fn spatha_frame() {
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let [first, _, _] = pushes(Spatha, 3, vec![], &a, &[]);
        assert_eq!(
            first,
            vec![Op::F(pad(
                &[0x10, 0, 0xF0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
                33
            ))]
        );
    }

    // RGBController_AsusROGStrixEvolve: DeviceUpdateMode(Direct), then the colour registers
    #[test]
    fn evolve_start_and_frame() {
        let [first, _, _] = pushes(Evolve, 1, vec![], &[Color(10, 20, 30)], &[]);
        let up = |k: u8, v: u8| {
            vec![
                Op::F(vec![0x0C, 0xC4, 0x0F, 0, k, v, 0, 0, 0]),
                Op::GetF(0x0C),
                Op::Sleep(15),
            ]
        };
        let want: Vec<Op> = [
            up(0x19, 1),
            up(0x1A, 255),
            up(0x1C, 10),
            up(0x1D, 20),
            up(0x1E, 30),
        ]
        .concat();
        assert_eq!(first, want);
    }
}
