// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "annepro2";
const LED_COUNT: usize = 61;
const REAL_COUNT: usize = 70;
const MESSAGE: usize = 51;

const MAP: [&[u32]; 5] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
    &[14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
    &[28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, NA],
    &[41, NA, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, NA],
    &[53, NA, 54, 55, NA, NA, 56, NA, NA, 57, 58, 59, 60, NA],
];

/// RGBController_AnnePro2::DeviceUpdateLEDs: 61 keys spread over 70 slots.
pub fn frame_buf(colors: &[Color]) -> [u8; REAL_COUNT * 3] {
    let mut buf = [0u8; REAL_COUNT * 3];
    let mut real = 0;
    for (i, c) in colors.iter().enumerate().take(LED_COUNT) {
        buf[real * 3..real * 3 + 3].copy_from_slice(&rgb(*c));
        real += match i {
            40 | 41 | 52 | 53 | 60 => 2,
            55 | 56 => 3,
            _ => 1,
        };
    }
    buf
}

/// AnnePro2Controller::SendDirect: five 64-byte messages of 51 data bytes, 50 ms apart.
pub fn messages(frame: &[u8]) -> Vec<[u8; 64]> {
    let count = frame.len() / MESSAGE + 1;
    let rest = frame.len() % MESSAGE;
    let last = if rest == 0 { MESSAGE } else { rest };
    (0..count)
        .map(|p| {
            let mut m = [0u8; 64];
            m[..4].copy_from_slice(&[0, 123, 16, 65]);
            m[4] = ((count << 4) + p) as u8;
            m[5] = (if p == count - 1 { last } else { MESSAGE } + 4) as u8;
            m[6..13].copy_from_slice(&[0, 0, 125, 32, 3, 255, 2]);
            for i in 0..MESSAGE {
                m[13 + i] = frame.get(p * MESSAGE + i).copied().unwrap_or(0);
            }
            m
        })
        .collect()
}

struct AnnePro2;

impl Proto for AnnePro2 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for m in messages(&frame_buf(colors)) {
            io.write(&m)?;
            io.sleep(50);
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (_, leds, grid) = mapped(&MAP);
    let dev = placed(info, FAMILY, d, DeviceKind::Keyboard, grid, leds);
    Ok(boxed(opened(api, info)?, FAMILY, dev, AnnePro2))
}

pub static DETECTORS: [Detector; 5] = [
    det("Anne Pro 2", 0x04D9, 0x8008, 1, ANY, ANY, open),
    det("Anne Pro 2", 0x04D9, 0x8009, 1, ANY, ANY, open),
    det("Anne Pro 2", 0x04D9, 0xA292, 1, ANY, ANY, open),
    det("Anne Pro 2", 0x04D9, 0xA293, 1, ANY, ANY, open),
    det("Anne Pro 2", 0x3311, 0xA297, 1, ANY, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn map_covers_every_led() {
        let (ids, _, _) = mapped(&MAP);
        assert_eq!(ids, (0..LED_COUNT as u32).collect::<Vec<_>>());
    }

    // RGBController_AnnePro2::DeviceUpdateLEDs gaps after LEDs 40, 41, 52, 53, 55, 56, 60
    #[test]
    fn frame_skips_gap_slots() {
        let colors: Vec<Color> = (0..61u8).map(|i| Color(i, i, i)).collect();
        let b = frame_buf(&colors);
        assert_eq!(&b[40 * 3..41 * 3], &[40; 3]);
        assert_eq!(&b[41 * 3..42 * 3], &[0; 3]);
        assert_eq!(&b[42 * 3..43 * 3], &[41; 3]);
        assert_eq!(&b[44 * 3..45 * 3], &[42; 3]);
        assert_eq!(&b[54 * 3..55 * 3], &[52; 3]);
        assert_eq!(&b[55 * 3..56 * 3], &[0; 3]);
        assert_eq!(&b[56 * 3..57 * 3], &[53; 3]);
        assert_eq!(&b[58 * 3..59 * 3], &[54; 3]);
        assert_eq!(&b[59 * 3..60 * 3], &[55; 3]);
        assert_eq!(&b[62 * 3..63 * 3], &[56; 3]);
        assert_eq!(&b[65 * 3..66 * 3], &[57; 3]);
        assert_eq!(&b[68 * 3..69 * 3], &[60; 3]);
        assert_eq!(&b[69 * 3..70 * 3], &[0; 3]);
    }

    // AnnePro2Controller::SendDirect(210, frame)
    #[test]
    fn direct_messages() {
        let mut colors = vec![Color::BLACK; 61];
        colors[0] = Color(1, 2, 3);
        colors[60] = Color(7, 8, 9);
        let [first, again, _] = pushes(AnnePro2, 61, vec![], &colors, &colors);
        assert!(again.is_empty());
        assert_eq!(first.len(), 10);
        let head = |e: u8, a: u8| vec![0, 123, 16, 65, e, a, 0, 0, 125, 32, 3, 255, 2];
        let mut m0 = head(0x50, 55);
        m0.extend_from_slice(&[1, 2, 3]);
        assert_eq!(first[0], w(pad(&m0, 64)));
        assert_eq!(first[1], Op::Sleep(50));
        for p in 1..4u8 {
            assert_eq!(first[p as usize * 2], w(pad(&head(0x50 + p, 55), 64)));
        }
        let mut m4 = head(0x54, 10);
        m4.extend_from_slice(&[7, 8, 9]);
        assert_eq!(first[8], w(pad(&m4, 64)));
        assert_eq!(first[9], Op::Sleep(50));
    }
}
