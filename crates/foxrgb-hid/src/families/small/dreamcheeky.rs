//! Ported from OpenRGB Controllers/DreamCheekyController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "dreamcheeky";

pub const INIT: [[u8; 9]; 4] = [
    [0x00, 0x1F, 0x02, 0x00, 0x5F, 0x00, 0x00, 0x1F, 0x03],
    [0x00, 0x00, 0x02, 0x00, 0x5F, 0x00, 0x00, 0x1F, 0x04],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F, 0x05],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
];

fn level(v: u8) -> u8 {
    (v >> 2) + (v == 255) as u8
}

pub fn set_color(c: Color) -> [u8; 9] {
    [0, level(c.0), level(c.1), level(c.2), 0, 0, 0, 0x1F, 0x05]
}

struct DreamCheeky;

impl Proto for DreamCheeky {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        INIT.iter().try_for_each(|p| io.write(p))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&set_color(colors[0]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    boxed(api, info, FAMILY, dev, DreamCheeky)
}

pub static DETECTORS: &[Detector] = &[det(
    "Dream Cheeky Webmail Notifier",
    0x1D34,
    0x0004,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DreamCheekyController constructor, then SetColor
    #[test]
    fn init_then_colour() {
        let [first, _, next] = pushes(
            DreamCheeky,
            1,
            vec![],
            &[Color(255, 4, 7)],
            &[Color(0, 254, 128)],
        );
        let mut e: Vec<Op> = INIT.iter().map(|p| w(p.to_vec())).collect();
        e.push(w(vec![0, 64, 1, 1, 0, 0, 0, 0x1F, 0x05]));
        assert_eq!(first, e);
        assert_eq!(next, vec![w(vec![0, 0, 63, 32, 0, 0, 0, 0x1F, 0x05])]);
    }
}
