//! Ported from OpenRGB Controllers/WitmodKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::klm::{FULL, Klm, Layout, TKL};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "witmod";
const LEN: usize = 64;
const INFO: u8 = 0x0D;
const CUSTOM: u8 = 0x0A;
const BRIGHTNESS_MAX: u8 = 0x04;

const GK8110: &[u32] = &[
    7, 13, 16, 19, 22, 28, 31, 34, 37, 40, 43, 46, 49, 52, 55, 58, 83, 86, 89, 92, 95, 98, 101,
    104, 107, 110, 113, 116, 119, 135, 138, 141, 144, 159, 162, 165, 168, 171, 174, 177, 180, 183,
    186, 199, 202, 205, 211, 214, 217, 220, 235, 241, 244, 247, 250, 263, 266, 269, 272, 275, 278,
    281, 284, 287, 311, 314, 327, 330, 333, 336, 339, 342, 345, 348, 351, 354, 363, 369, 397, 400,
    403, 415, 427, 430, 433, 436, 442, 455, 458,
];

const GK8120: &[u32] = &[
    7, 13, 16, 19, 22, 28, 31, 34, 37, 40, 43, 46, 49, 52, 55, 58, 83, 86, 89, 92, 95, 98, 101,
    104, 107, 110, 113, 116, 119, 135, 138, 141, 144, 147, 150, 153, 156, 159, 162, 165, 168, 171,
    174, 177, 180, 183, 186, 199, 202, 205, 211, 214, 217, 220, 223, 226, 229, 232, 235, 241, 244,
    247, 250, 263, 266, 269, 272, 275, 278, 281, 284, 287, 299, 302, 305, 311, 314, 327, 330, 333,
    336, 339, 342, 345, 348, 351, 354, 363, 369, 375, 378, 391, 394, 397, 400, 403, 415, 427, 430,
    433, 436, 442, 455, 458, 461, 467,
];

fn keyboard(full: bool) -> Klm {
    if full {
        Klm::new(Layout::AnsiQwerty, FULL, GK8120, &[])
    } else {
        Klm::new(Layout::AnsiQwerty, TKL, GK8110, &[])
    }
}

pub fn info_request() -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[0] = 0x01;
    b[1] = INFO;
    b
}

/// ReadDeviceInfo: true for a GK8120 (full size), false otherwise.
pub fn read_full_size(io: &mut dyn Io) -> Result<bool, String> {
    io.write(&info_request())?;
    for _ in 0..10 {
        let mut reply = [0u8; LEN];
        if io.read(&mut reply, 150)? == 0 || reply[0] != 0x01 || reply[1] != INFO {
            continue;
        }
        let text: Vec<u8> = reply[2..]
            .iter()
            .copied()
            .skip_while(|&b| b <= b' ')
            .collect();
        let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
        if end == 0 {
            continue;
        }
        let text = String::from_utf8_lossy(&text[..end]);
        return Ok(text.contains("GK8120"));
    }
    Ok(false)
}

/// SetMode for Custom: no colours, full brightness, direction left.
pub fn custom_mode() -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[0] = 0x01;
    b[1] = 0x07;
    b[6] = CUSTOM;
    b[7] = BRIGHTNESS_MAX;
    b
}

pub fn leds_data(values: &[u32], colors: &[Color]) -> Vec<[u8; LEN]> {
    let mut data = [0u8; LEN * 8];
    for (&v, c) in values.iter().zip(colors) {
        let o = v as usize;
        data[o - 1..o + 2].copy_from_slice(&rgb(*c));
    }
    (0..8)
        .map(|i| {
            let mut p = [0u8; LEN];
            p[0] = 0x01;
            p[1] = 0x0F;
            p[4] = i as u8;
            p[5] = if i == 7 { 0x12 } else { 0x36 };
            p[6..].copy_from_slice(&data[LEN * i + 6..LEN * (i + 1)]);
            p
        })
        .collect()
}

struct Witmod {
    values: Vec<u32>,
}

impl Proto for Witmod {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&custom_mode())?;
        io.sleep(10);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in leds_data(&self.values, colors) {
            io.write(&p)?;
        }
        io.sleep(10);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let kb = keyboard(read_full_size(io.as_mut())?);
    let dev = kb.device(info, FAMILY, d);
    Ok(Box::new(Driver::new(
        io,
        dev,
        FAMILY,
        Witmod {
            values: kb.values(),
        },
    )))
}

pub static DETECTORS: &[Detector] = &[det("Witmod Keyboard", 0x0416, 0xC345, 2, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // ReadDeviceInfo picks the model from the reply text
    #[test]
    fn model_from_info_reply() {
        let mut reply = vec![0x01, INFO, b' ', b' '];
        reply.extend(b"GK8120 V1.02");
        let mut rec = Rec::with(vec![vec![0x02, INFO], reply]);
        assert!(read_full_size(&mut rec).unwrap());
        assert_eq!(
            rec.take(),
            vec![w(info_request().to_vec()), Op::Read, Op::Read]
        );
        let mut rec = Rec::with(vec![]);
        assert!(!read_full_size(&mut rec).unwrap());
        assert_eq!(rec.take().len(), 11);
    }

    #[test]
    fn layouts() {
        assert_eq!(keyboard(false).keys.len(), 87);
        assert_eq!(keyboard(true).keys.len(), 104);
    }

    // SetMode(Custom) then SetLEDsData
    #[test]
    fn custom_then_led_data() {
        let kb = keyboard(false);
        let n = kb.keys.len();
        let mut a = vec![Color::BLACK; n];
        a[0] = Color(1, 2, 3); // Escape, offset 7
        let [first, _, _] = pushes(
            Witmod {
                values: kb.values(),
            },
            n,
            vec![],
            &a,
            &[],
        );
        assert_eq!(first.len(), 2 + 8 + 1);
        assert_eq!(first[0], w(pad(&[1, 7, 0, 0, 0, 0, 0x0A, 4], 64)));
        assert_eq!(first[1], Op::Sleep(10));
        assert_eq!(first[2], w(pad(&[1, 0x0F, 0, 0, 0, 0x36, 1, 2, 3], 64)));
        let Op::W(last) = &first[9] else { panic!() };
        assert_eq!(last[..6], [1, 0x0F, 0, 0, 7, 0x12]);
        assert_eq!(first[10], Op::Sleep(10));
    }
}
