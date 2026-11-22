// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, at, device, on_i, open_with};
use crate::registry::{Detector, Info};

const VID: u16 = 0x0951;
const VID_HP: u16 = 0x03F0;

/// `HyperXMicrophoneController::SendDirect`: top and bottom colour.
fn direct(top: Color, bottom: Color) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1..9].copy_from_slice(&[
        0x81, top.0, top.1, top.2, 0x81, bottom.0, bottom.1, bottom.2,
    ]);
    b
}

/// `HyperXMicrophoneController::SendToRegister`.
fn register(reg: u8, param1: u8, param2: u8) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = 0x04;
    b[2] = reg;
    b[8] = param1;
    b[9] = param2;
    b
}

struct Quadcast;

impl Protocol for Quadcast {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![
            Op::Feature(direct(at(colors, 0), at(colors, 1))),
            Op::Sleep(15),
            Op::Feature(register(0xF2, 0, 1)),
            Op::Sleep(15),
        ]
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_millis(50))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "hyperx microphone", |_| {
        let dev = device(
            info.device_id("hyperx-microphone"),
            d.name,
            DeviceKind::Other,
            [1, 2],
            vec![[0.5, 0.25], [0.5, 0.75]],
        );
        Ok((dev, Quadcast))
    })
}

pub(super) static DETECTORS: [Detector; 9] = [
    on_i("HyperX Quadcast S", VID, 0x171D, 0, open),
    on_i("HyperX Quadcast S", VID, 0x171F, 0, open),
    on_i("HyperX Quadcast S", VID_HP, 0x0F8B, 0, open),
    on_i("HyperX Quadcast S", VID_HP, 0x068C, 0, open),
    on_i("HyperX Quadcast S", VID_HP, 0x0294, 0, open),
    on_i("HyperX Quadcast S", VID_HP, 0x028C, 0, open),
    on_i("HyperX Quadcast S", VID_HP, 0x048C, 0, open),
    on_i("HyperX Quadcast S", VID_HP, 0x0D8B, 0, open),
    on_i("HyperX DuoCast", VID_HP, 0x098C, 0, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::buf;
    use super::*;

    // HyperXMicrophoneController::SendDirect, then SendToRegister(0xF2, 0, 1)
    #[test]
    fn frame_sends_colours_then_apply() {
        let ops = Quadcast.frame(&[Color(1, 2, 3), Color(4, 5, 6)]);
        let colours = buf(65, &[0x00, 0x81, 1, 2, 3, 0x81, 4, 5, 6]);
        let apply = buf(65, &[0x00, 0x04, 0xF2, 0, 0, 0, 0, 0, 0x00, 0x01]);
        assert_eq!(
            ops,
            vec![
                Op::Feature(colours),
                Op::Sleep(15),
                Op::Feature(apply),
                Op::Sleep(15)
            ]
        );
        assert_eq!(Quadcast.keepalive(), Some(Duration::from_millis(50)));
    }
}
