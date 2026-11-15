//! Ported from OpenRGB Controllers/HyperXMousematController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, at, device, on_i, on_ipu, open_with};
use crate::registry::{Detector, Info};

const FURY_A_XL: u16 = 0x1741;

/// `HyperXMousematController::SendDirect`: a setup report, then two of 16 colours. OpenRGB reads
/// 32 colours from a 20- or 2-LED buffer; the missing ones go out black here.
fn direct(colors: &[Color]) -> [Vec<u8>; 3] {
    let mut setup = vec![0u8; 65];
    setup[1] = 0x04;
    setup[2] = 0xF2;
    setup[9] = 0x02;
    let half = |from: usize| {
        let mut b = vec![0u8; 65];
        for i in 0..16 {
            let c = at(colors, from + i);
            b[1 + i * 4..5 + i * 4].copy_from_slice(&[0x81, c.0, c.1, c.2]);
        }
        b
    };
    [setup, half(0), half(16)]
}

struct Mat {
    leds: usize,
}

impl Protocol for Mat {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        direct(&colors[..colors.len().min(self.leds)])
            .into_iter()
            .map(Op::Feature)
            .collect()
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_millis(50))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "hyperx mousemat", |_| {
        // Underglow, then the LED strip.
        let zones: &[usize] = if info.pid == FURY_A_XL {
            &[2]
        } else {
            &[15, 5]
        };
        let leds: Vec<[f32; 2]> = zones
            .iter()
            .enumerate()
            .flat_map(|(z, &n)| {
                let y = if z == 0 { 0.9 } else { 0.1 };
                (0..n).map(move |i| [(i as f32 + 0.5) / n as f32, y])
            })
            .collect();
        let n = leds.len();
        let mut dev = device(
            info.device_id("hyperx-mousemat"),
            d.name,
            DeviceKind::Other,
            [15, 2],
            leds,
        );
        dev.rect = [0.62, 0.58, 0.32, 0.34];
        Ok((dev, Mat { leds: n }))
    })
}

pub(super) static DETECTORS: [Detector; 3] = [
    on_i("HyperX Fury Ultra", 0x0951, 0x1705, 0, open),
    on_ipu(
        "HyperX Pulsefire Mat",
        0x03F0,
        0x0F8D,
        1,
        0xFF90,
        0xFF00,
        open,
    ),
    on_ipu(
        "HyperX Pulsefire Mat RGB Mouse Pad XL",
        0x0951,
        FURY_A_XL,
        1,
        0xFF90,
        0xFF00,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::buf;
    use super::*;

    // HyperXMousematController::SendDirect with 20 LEDs; slots 20..31 are black
    #[test]
    fn frame_sends_setup_and_two_halves() {
        let colors: Vec<Color> = (0..20).map(|i| Color(i, 50, 60)).collect();
        let ops = Mat { leds: 20 }.frame(&colors);
        let mut setup = buf(65, &[0x00, 0x04, 0xF2]);
        setup[9] = 0x02;
        let mut a = vec![0u8; 65];
        for i in 0..16u8 {
            let at = 1 + i as usize * 4;
            a[at..at + 4].copy_from_slice(&[0x81, i, 50, 60]);
        }
        let mut b = vec![0u8; 65];
        for i in 0..16u8 {
            let at = 1 + i as usize * 4;
            let c = if i < 4 {
                [0x81, 16 + i, 50, 60]
            } else {
                [0x81, 0, 0, 0]
            };
            b[at..at + 4].copy_from_slice(&c);
        }
        assert_eq!(
            ops,
            vec![Op::Feature(setup), Op::Feature(a), Op::Feature(b)]
        );
    }
}
