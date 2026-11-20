//! Ported from OpenRGB Controllers/HyperXMicrophoneV2Controller (GPL-2.0-or-later).

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, at, device, matrix, on_ipu, open_with};
use crate::registry::{Detector, Info};

const LEDS: usize = 108;
const REPORT_ID: u8 = 0x44;
const PER_PACKET: usize = 20;
/// `WaitForResponse` waits this long for each echo.
const READ: Op = Op::Read(64, 2000);

/// Front then rear, 9 rows of 12; positions only, colours go out in LED order.
#[rustfmt::skip]
const MAP: [u32; LEDS] = [
    26, 27, 44, 45, 62, 63, 80, 81, 98, 99, 8, 9,
    25, 28, 43, 46, 61, 64, 79, 82, 97, 100, 7, 10,
    24, 29, 42, 47, 60, 65, 78, 83, 96, 101, 6, 11,
    23, 30, 41, 48, 59, 66, 77, 84, 95, 102, 5, 12,
    22, 31, 40, 49, 58, 67, 76, 85, 94, 103, 4, 13,
    21, 32, 39, 50, 57, 68, 75, 86, 93, 104, 3, 14,
    20, 33, 38, 51, 56, 69, 74, 87, 92, 105, 2, 15,
    19, 34, 37, 52, 55, 70, 73, 88, 91, 106, 1, 16,
    18, 35, 36, 53, 54, 71, 72, 89, 90, 107, 0, 17,
];

/// `HyperXMicrophoneV2Controller::SendDirect`: the `01 06` header, then six colour packets.
fn direct(colors: &[Color]) -> Vec<Op> {
    let mut head = vec![0u8; 64];
    head[..3].copy_from_slice(&[REPORT_ID, 0x01, 0x06]);
    let mut ops = vec![Op::Write(head), READ];
    for packet in 0..6 {
        let mut b = vec![0u8; 64];
        b[..3].copy_from_slice(&[REPORT_ID, 0x02, packet as u8]);
        let from = packet * PER_PACKET;
        for (slot, led) in (from..(from + PER_PACKET).min(LEDS)).enumerate() {
            let c = at(colors, led);
            b[4 + slot * 3..7 + slot * 3].copy_from_slice(&[c.0, c.1, c.2]);
        }
        ops.push(Op::Write(b));
        ops.push(READ);
    }
    ops
}

struct Quadcast2S;

impl Protocol for Quadcast2S {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        direct(colors)
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "hyperx quadcast 2 s", |_| {
        let leds = matrix(&MAP, 12, LEDS, |_| [0.5, 0.5]);
        let dev = device(
            info.device_id("hyperx-quadcast-2s"),
            d.name,
            DeviceKind::Other,
            [12, 9],
            leds,
        );
        Ok((dev, Quadcast2S))
    })
}

pub(super) static DETECTORS: [Detector; 1] = [on_ipu(
    "HyperX QuadCast 2 S",
    0x03F0,
    0x02B5,
    1,
    0xFF13,
    0xFF00,
    open,
)];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    // HyperXMicrophoneV2Controller::SendDirect + SendColorPackets(colors, 0x02)
    #[test]
    fn frame_sends_header_and_six_packets() {
        let colors: Vec<Color> = (0..LEDS).map(|i| Color(i as u8, 1, 2)).collect();
        let ops = Quadcast2S.frame(&colors);
        assert_eq!(ops.len(), 14);
        assert!(
            ops.iter()
                .skip(1)
                .step_by(2)
                .all(|o| *o == Op::Read(64, 2000))
        );
        assert_eq!(bytes(&ops[0]), buf(64, &[0x44, 0x01, 0x06]).as_slice());
        let p0 = bytes(&ops[2]);
        assert_eq!(p0[..7], [0x44, 0x02, 0x00, 0x00, 0, 1, 2]);
        assert_eq!(p0[61..64], [19, 1, 2]);
        let p5 = bytes(&ops[12]);
        assert_eq!(p5[..7], [0x44, 0x02, 0x05, 0x00, 100, 1, 2]);
        assert_eq!(p5[25..28], [107, 1, 2]);
        assert!(p5[28..].iter().all(|&b| b == 0));
    }

    #[test]
    fn map_covers_every_led() {
        for led in 0..LEDS as u32 {
            assert!(MAP.contains(&led));
        }
    }
}
