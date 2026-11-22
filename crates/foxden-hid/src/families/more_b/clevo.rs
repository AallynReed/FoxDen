// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, at, backend, device, on_pu, open_io};
use crate::registry::{Detector, Info};

const ROWS: usize = 6;
const MODE_DIRECT: u8 = 0x33;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chip {
    Ite8291,
    Ite829x,
}

impl Chip {
    fn cols(self) -> usize {
        match self {
            Chip::Ite8291 => 21,
            Chip::Ite829x => 20,
        }
    }
    /// The Direct mode's brightness, its maximum.
    fn brightness(self) -> u8 {
        match self {
            Chip::Ite8291 => 0x32,
            Chip::Ite829x => 0x0A,
        }
    }
}

/// `ClevoKeyboardController::SendColors` in Direct mode. LEDs are the hardware grid in
/// `row * cols + col` order, row 0 being the bottom row.
fn send_colors(io: &mut dyn Io, chip: Chip, colors: &[Color]) -> Result<(), String> {
    let cols = chip.cols();
    match chip {
        Chip::Ite829x => {
            io.feature(&[0xCC, 0x09, chip.brightness(), 0x00, 0x00, 0x00])?;
            for row in 0..ROWS {
                for col in 0..cols {
                    let c = at(colors, row * cols + col);
                    let id = (((row & 7) << 5) | (col & 0x1F)) as u8;
                    io.feature(&[0xCC, 0x01, id, c.0, c.1, c.2])?;
                }
            }
        }
        Chip::Ite8291 => {
            io.feature(&[0x08, 0x02, MODE_DIRECT, 0x00, chip.brightness(), 0, 0, 0])?;
            for row in 0..ROWS {
                io.feature(&[0x16, 0x00, row as u8, 0, 0, 0, 0, 0])?;
                let mut b = vec![0u8; 65];
                for col in 0..cols {
                    let c = at(colors, row * cols + col);
                    b[2 + col] = c.2;
                    b[2 + cols + col] = c.1;
                    b[2 + cols * 2 + col] = c.0;
                }
                io.write(&b)?;
            }
        }
    }
    Ok(())
}

struct Clevo {
    chip: Chip,
}

impl Driver for Clevo {
    // RGBController_ClevoKeyboard::DeviceUpdateLEDs with Direct active.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send_colors(io, self.chip, colors)
    }
}

/// Hardware grid positions, row 0 drawn at the bottom.
fn leds(chip: Chip) -> Vec<[f32; 2]> {
    let cols = chip.cols();
    (0..ROWS * cols)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            [
                (col as f32 + 0.5) / cols as f32,
                ((ROWS - 1 - row) as f32 + 0.5) / ROWS as f32,
            ]
        })
        .collect()
}

fn open(api: &HidApi, info: &Info, d: &Detector, chip: Chip) -> Result<Box<dyn Backend>, String> {
    let io = open_io(api, info)?;
    let dev = device(
        info.device_id("clevo"),
        d.name,
        DeviceKind::Keyboard,
        [chip.cols() as u32, ROWS as u32],
        leds(chip),
    );
    Ok(backend(io, dev, Clevo { chip }, "clevo keyboard"))
}

fn open_8291(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Chip::Ite8291)
}

fn open_829x(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Chip::Ite829x)
}

pub(super) static DETECTORS: [Detector; 2] = [
    on_pu(
        "Clevo Keyboard (ITE 8291)",
        0x048D,
        0x600B,
        0xFF03,
        0x01,
        open_8291,
    ),
    on_pu(
        "Clevo Keyboard (ITE 829x)",
        0x048D,
        0x8910,
        0xFF89,
        0xCC,
        open_829x,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op};
    use super::*;

    fn colors(n: usize) -> Vec<Color> {
        (0..n).map(|i| Color(i as u8, 100 + i as u8, 200)).collect()
    }

    // ClevoKeyboardController::SendColors for the ITE 8291: mode, then per row a select and BGR planes
    #[test]
    fn ite8291_frame() {
        let mut fake = Fake::default();
        Clevo {
            chip: Chip::Ite8291,
        }
        .frame(&mut fake, &colors(126))
        .unwrap();
        let ops = fake.ops();
        assert_eq!(ops.len(), 1 + 6 * 2);
        assert_eq!(
            ops[0],
            Op::Feature(vec![0x08, 0x02, 0x33, 0x00, 0x32, 0, 0, 0])
        );
        assert_eq!(ops[5], Op::Feature(vec![0x16, 0x00, 2, 0, 0, 0, 0, 0]));
        let mut row2 = vec![0u8; 65];
        for col in 0..21 {
            let i = 42 + col as u8;
            row2[2 + col] = 200;
            row2[23 + col] = 100 + i;
            row2[44 + col] = i;
        }
        assert_eq!(ops[6], Op::Write(row2));
    }

    // ClevoKeyboardController::SendColors for the ITE 829x: brightness, then one report per LED
    #[test]
    fn ite829x_frame() {
        let mut fake = Fake::default();
        Clevo {
            chip: Chip::Ite829x,
        }
        .frame(&mut fake, &colors(120))
        .unwrap();
        let ops = fake.ops();
        assert_eq!(ops.len(), 121);
        assert_eq!(ops[0], Op::Feature(vec![0xCC, 0x09, 0x0A, 0, 0, 0]));
        assert_eq!(ops[1], Op::Feature(vec![0xCC, 0x01, 0x00, 0, 100, 200]));
        // row 5, col 3 is LED 103, id 0xA3
        assert_eq!(ops[104], Op::Feature(vec![0xCC, 0x01, 0xA3, 103, 203, 200]));
    }

    #[test]
    fn row_zero_is_drawn_at_the_bottom() {
        let l = leds(Chip::Ite8291);
        assert_eq!(l[105], [0.5 / 21.0, 0.5 / 6.0]);
        assert_eq!(l[0], [0.5 / 21.0, 5.5 / 6.0]);
    }
}
