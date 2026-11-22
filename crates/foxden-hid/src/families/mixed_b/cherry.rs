// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{NA, Op, Protocol, at, device, matrix, on_ip, open_with};
use crate::registry::{Detector, Info};

const VID: u16 = 0x046A;
const PAGE: u16 = 0xFF1C;
const WIDTH: usize = 21;
const HEIGHT: usize = 6;
const CELLS: usize = WIDTH * HEIGHT;
const MAX_DATA: usize = 0x36;
const MODE_CUSTOM: u8 = 0x08;
const BRIGHTNESS_HIGHEST: u8 = 0x04;
const READ: Op = Op::Read(64, 500);

#[rustfmt::skip]
const MAP: [u32; CELLS] = [
    0, NA, 12, 18, 24, 30, 36, 42, 48, 54, 60, 66, 72, 78, 84, 90, 96, 102, 108, 114, 120,
    1, 7, 13, 19, 25, 31, 37, 43, 49, 55, 61, 67, 73, 79, 85, 91, 97, 103, 109, 115, 121,
    2, 8, 14, 20, 26, 32, 38, 44, 50, 56, 62, 68, 74, 80, 86, 92, 98, 104, 110, 116, NA,
    3, 9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69, 75, 81, NA, NA, NA, 105, 111, 117, 122,
    4, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, NA, 82, NA, 94, NA, 106, 112, 118, NA,
    5, 11, 17, NA, NA, NA, 41, NA, NA, NA, 65, 71, 77, 83, 89, 95, 101, 113, NA, 119, 124,
];

/// `ComputeChecksum`: the sum of bytes 3..64 as signed `char`s, which they are on Windows.
fn checksum(b: &mut [u8]) {
    let sum = b[3..64]
        .iter()
        .fold(0u16, |s, &x| s.wrapping_add(x as i8 as i16 as u16));
    b[1] = sum as u8;
    b[2] = (sum >> 8) as u8;
}

/// `SendKeyboardBegin` / `SendKeyboardEnd`.
fn marker(command: u8) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..4].copy_from_slice(&[0x04, command, 0x00, command]);
    b
}

/// `SendKeyboardParameter`.
fn parameter(parameter: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x04;
    b[3] = 0x06;
    b[4] = data.len() as u8;
    b[5] = parameter;
    b[7] = 0x55;
    b[8..8 + data.len()].copy_from_slice(data);
    checksum(&mut b);
    b
}

/// `SendKeyboardData`.
fn data(chunk: &[u8], offset: usize) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x04;
    b[3] = 0x0B;
    b[4] = chunk.len() as u8;
    b[5] = offset as u8;
    b[6] = (offset >> 8) as u8;
    b[8..8 + chunk.len()].copy_from_slice(chunk);
    checksum(&mut b);
    b
}

struct Cherry;

impl Protocol for Cherry {
    /// `DeviceUpdateMode` for Custom: `SendKeyboardModeEx(0x08, 4, 0, 0, 0, 0, 0, 0)`.
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        let mode = [0x00, MODE_CUSTOM, BRIGHTNESS_HIGHEST, 0, 0, 0, 0, 0, 0];
        vec![Op::Write(parameter(0x00, &mode)), READ]
    }

    /// `DeviceUpdateLEDs`: begin, 378 colour bytes in blocks of 54, end; each answered.
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let bytes: Vec<u8> = (0..CELLS)
            .flat_map(|i| {
                let c = at(colors, i);
                [c.0, c.1, c.2]
            })
            .collect();
        let mut packets = vec![marker(0x01)];
        for (i, chunk) in bytes.chunks(MAX_DATA).enumerate() {
            packets.push(data(chunk, i * MAX_DATA));
        }
        packets.push(marker(0x02));
        packets
            .into_iter()
            .flat_map(|p| [Op::Write(p), READ])
            .collect()
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "cherry keyboard", |_| {
        let leds = matrix(&MAP, WIDTH, CELLS, |i| {
            [
                ((i / HEIGHT) as f32 + 0.5) / WIDTH as f32,
                ((i % HEIGHT) as f32 + 0.5) / HEIGHT as f32,
            ]
        });
        let dev = device(
            info.device_id("cherry"),
            d.name,
            DeviceKind::Keyboard,
            [WIDTH as u32, HEIGHT as u32],
            leds,
        );
        Ok((dev, Cherry))
    })
}

macro_rules! cherry {
    ($($name:literal $pid:literal),* $(,)?) => {
        pub(super) static DETECTORS: [Detector; 25] = [$(on_ip($name, VID, $pid, 1, PAGE, open)),*];
    };
}

cherry! {
    "Cherry Keyboard MX BOARD 3.0S FL NBL" 0x0077,
    "Cherry Keyboard MX BOARD 3.0S FL RGB" 0x0079,
    "Cherry Keyboard MX BOARD 3.0S FL RGB KOREAN" 0x0083,
    "Cherry Keyboard MX 1.0 FL BL" 0x00AB,
    "Cherry Keyboard MX BOARD 1.0 TKL RGB" 0x00AC,
    "Cherry Keyboard MX BOARD 8.0 TKL RGB" 0x00B7,
    "Cherry Keyboard MX BOARD 10.0 FL RGB" 0x00BB,
    "Cherry Keyboard G80-3000 TKL NBL" 0x00C3,
    "Cherry Keyboard MX BOARD 2.0S FL RGB (EU)" 0x01A6,
    "Cherry Keyboard MX BOARD 2.0S FL RGB (US)" 0x00C4,
    "Cherry Keyboard MX BOARD 2.0S FL NBL" 0x00CE,
    "Cherry Keyboard G80-3000 TKL RGB" 0x00C5,
    "Cherry Keyboard MV BOARD 3.0 FL RGB" 0x00C7,
    "Cherry Keyboard CCF MX 8.0 TKL BL" 0x00C9,
    "Cherry Keyboard CCF MX 1.0 TKL BL" 0x00CA,
    "Cherry Keyboard CCF MX 1.0 TKL NBL" 0x00CB,
    "Cherry Keyboard G80-3000 TKL NBL KOREAN" 0x00CD,
    "Cherry Keyboard MX 1.0 FL NBL" 0x00D2,
    "Cherry Keyboard MX 1.0 FL RGB" 0x00D3,
    "Cherry Keyboard G80-3000N TKL RGB (EU)" 0x00DD,
    "Cherry Keyboard G80-3000N TKL RGB (US)" 0x00E0,
    "Cherry Keyboard G80-3000N FL RGB (EU)" 0x00DE,
    "Cherry Keyboard G80-3000N FL RGB (US)" 0x00E1,
    "Cherry Keyboard MX BOARD 10.0N FL RGB (EU)" 0x00DF,
    "Cherry Keyboard MX BOARD 10.0N FL RGB (US)" 0x00E2,
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    // DeviceUpdateMode(Custom) -> SendKeyboardModeEx -> SendKeyboardParameter(0, 9, ...)
    #[test]
    fn setup_selects_custom_mode() {
        let ops = Cherry.setup(&[]);
        // checksum: 0x06 + 9 + 0x55 + 0x08 + 0x04 = 0x70
        let want = buf(
            64,
            &[
                0x04, 0x70, 0x00, 0x06, 0x09, 0x00, 0x00, 0x55, 0x00, 0x08, 0x04,
            ],
        );
        assert_eq!(ops, vec![Op::Write(want), Op::Read(64, 500)]);
    }

    // ComputeChecksum adds signed chars: 0xFF counts as -1
    #[test]
    fn checksum_is_signed() {
        let p = data(&[0xFF, 0x80, 0x01], 0x0136);
        // 0x0B + 3 + 0x36 + 0x01 - 1 - 128 + 1 = -59
        let sum = (-59i16) as u16;
        assert_eq!(
            p[..9],
            [
                0x04,
                sum as u8,
                (sum >> 8) as u8,
                0x0B,
                3,
                0x36,
                0x01,
                0x00,
                0xFF
            ]
        );
    }

    // DeviceUpdateLEDs: SendKeyboardBegin, SetKeyboardColors (7 packets), SendKeyboardEnd
    #[test]
    fn frame_wraps_colour_data() {
        let colors: Vec<Color> = (0..CELLS).map(|i| Color(i as u8, 0, 0)).collect();
        let ops = Cherry.frame(&colors);
        assert_eq!(ops.len(), 2 * 9);
        assert!(
            ops.iter()
                .skip(1)
                .step_by(2)
                .all(|o| *o == Op::Read(64, 500))
        );
        assert_eq!(
            bytes(&ops[0]),
            buf(64, &[0x04, 0x01, 0x00, 0x01]).as_slice()
        );
        assert_eq!(
            bytes(&ops[16]),
            buf(64, &[0x04, 0x02, 0x00, 0x02]).as_slice()
        );
        let first = bytes(&ops[2]);
        assert_eq!(first[3..8], [0x0B, 0x36, 0x00, 0x00, 0x00]);
        assert_eq!(first[8..14], [0, 0, 0, 1, 0, 0]);
        let last = bytes(&ops[14]);
        assert_eq!(last[3..7], [0x0B, 0x36, 0x44, 0x01]);
        assert_eq!(last[8 + 51..8 + 54], [125, 0, 0]);
        let mut check = last.to_vec();
        checksum(&mut check);
        assert_eq!(check, last);
    }

    #[test]
    fn unmapped_leds_fall_back_to_columns() {
        let leds = matrix(&MAP, WIDTH, CELLS, |i| [i as f32, -1.0]);
        assert_eq!(leds[123], [123.0, -1.0]);
        assert_eq!(leds[124][0], (20.0 + 0.5) / 21.0);
    }
}
