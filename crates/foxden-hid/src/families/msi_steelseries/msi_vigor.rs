// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Driver, Op, Proto, device, line};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const LEDS: usize = 6;
const CUSTOM_MODE: u8 = 0x70;
const OFF_MODE: u8 = 0x00;
const BRIGHTNESS_MAX: u8 = 0x03;

static DETECTORS: [Detector; 1] = [Detector {
    name: "MSI Vigor GK30 controller",
    vid: 0x0DB0,
    pid: 0x0B30,
    interface: Some(1),
    usage_page: Some(0xFF01),
    usage: Some(1),
    open,
}];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

/// [r][g][b] to palette index: red, yellow, green, cyan, blue, magenta, white.
const INDEX: [[[u8; 2]; 2]; 2] = [[[0x00, 0x04], [0x02, 0x03]], [[0x00, 0x05], [0x01, 0x06]]];

/// GetColourIndex: each channel divided (integer) by the largest one.
fn colour_index(c: Color) -> u8 {
    let largest = (c.0.max(c.1).max(c.2) as u32).max(1);
    let r = (c.0 as u32 / largest) as usize;
    let g = (c.1 as u32 / largest) as usize;
    let b = (c.2 as u32 / largest) as usize;
    INDEX[r][g][b]
}

/// SetMode for the Custom mode (per LED colour, brightness only).
fn custom_report(colors: &[Color], brightness: u8) -> [u8; 8] {
    let mut buf = [0u8; 8];
    buf[0] = 0x07;
    buf[1] = 0xFF;
    buf[2] = CUSTOM_MODE;
    let lit = colors.iter().take(LEDS).any(|&c| c != Color::BLACK);
    if !lit {
        buf[2] = OFF_MODE;
        return buf;
    }
    for i in 0..LEDS {
        let mut index = colour_index(colors.get(i).copied().unwrap_or(Color::BLACK));
        if i % 2 == 0 {
            index <<= 4;
        }
        buf[3 + i / 2] = buf[3 + i / 2].wrapping_add(index);
    }
    buf[6] = brightness;
    buf
}

struct Vigor;

impl Proto for Vigor {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![
            Op::Feature(custom_report(colors, BRIGHTNESS_MAX).to_vec()),
            Op::Sleep(10),
        ]
    }
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let dev = device(
        info.device_id("msi-vigor"),
        det.name,
        DeviceKind::Keyboard,
        line(LEDS),
    );
    Ok(Driver::boxed(hid, "msi vigor", dev, Vigor))
}

#[cfg(test)]
mod tests {
    use super::*;

    // MSIVigorGK30Controller::GetColourIndex.
    #[test]
    fn palette() {
        assert_eq!(colour_index(Color(255, 0, 0)), 0x00);
        assert_eq!(colour_index(Color(200, 200, 0)), 0x01);
        assert_eq!(colour_index(Color(0, 9, 0)), 0x02);
        assert_eq!(colour_index(Color(0, 9, 9)), 0x03);
        assert_eq!(colour_index(Color(0, 0, 1)), 0x04);
        assert_eq!(colour_index(Color(9, 0, 9)), 0x05);
        assert_eq!(colour_index(Color(7, 7, 7)), 0x06);
        assert_eq!(colour_index(Color(254, 255, 0)), 0x02);
    }

    // MSIVigorGK30Controller::SetMode with the Custom mode from RGBController_MSIVigorGK30.
    #[test]
    fn custom_frame() {
        let colors = [
            Color(255, 0, 0),
            Color(0, 255, 0),
            Color(0, 0, 255),
            Color(255, 255, 255),
            Color(255, 0, 255),
            Color(0, 0, 0),
        ];
        assert_eq!(
            custom_report(&colors, 3),
            [0x07, 0xFF, 0x70, 0x02, 0x46, 0x50, 0x03, 0x00]
        );
    }

    // All black switches to the off mode.
    #[test]
    fn all_black_is_off() {
        assert_eq!(
            custom_report(&[Color::BLACK; 6], 3),
            [0x07, 0xFF, 0x00, 0, 0, 0, 0, 0]
        );
        let ops = Vigor.frame(&[Color::BLACK; 6]);
        assert_eq!(ops[1], Op::Sleep(10));
    }
}
