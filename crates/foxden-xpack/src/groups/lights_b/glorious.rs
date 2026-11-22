//! Glorious Model O and Model D Wireless mice, on the dongle and on the cable.

use super::{MOUSE, boxed, det, placed};
use crate::hid::{Io, Proto, rgb};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "glorious-wireless";
const VID: u16 = 0x258A;
const LEN: usize = 65;
const COLOR_HEAD: [u8; 12] = [
    0, 0, 0, 0x02, 0x08, 0x02, 0x00, 0x01, 0xFF, 0x04, 0x00, 0x09,
];
/// Sent after every frame on the cable, and as the dongle's hand-back.
const LATCH: [[u8; 9]; 2] = [
    [0, 0, 0, 0x02, 0x02, 0x02, 0x02, 0x01, 0xFF],
    [0, 0, 0, 0x02, 0x02, 0x02, 0x02, 0x00, 0xFF],
];
const HARDWARE: [u8; 12] = [
    0, 0, 0, 0x02, 0x05, 0x02, 0x00, 0x01, 0xFF, 0x01, 0x00, 0x09,
];

/// (pid, name, LEDs driven, on the cable).
const MODELS: &[(u16, &str, usize, bool)] = &[
    (0x2011, "Glorious Model O Wireless (Wired)", 2, true),
    (0x2012, "Glorious Model D Wireless (Wired)", 1, true),
    (0x2013, "Glorious Model O Wireless (Wired)", 2, true),
    (0x2014, "Glorious Model D Wireless (Wired)", 1, true),
    (0x2022, "Glorious Model O Wireless", 2, false),
    (0x2023, "Glorious Model D Wireless", 1, false),
    (0x2024, "Glorious Model O Wireless", 2, false),
    (0x2025, "Glorious Model D Wireless", 1, false),
    (0x2026, "Glorious Model O Wireless", 2, false),
];

const fn mouse(pid: u16, name: &'static str) -> Detector {
    det(name, VID, pid, Some(2), None, None, open)
}

pub static DETECTORS: &[Detector] = &[
    mouse(MODELS[0].0, MODELS[0].1),
    mouse(MODELS[1].0, MODELS[1].1),
    mouse(MODELS[2].0, MODELS[2].1),
    mouse(MODELS[3].0, MODELS[3].1),
    mouse(MODELS[4].0, MODELS[4].1),
    mouse(MODELS[5].0, MODELS[5].1),
    mouse(MODELS[6].0, MODELS[6].1),
    mouse(MODELS[7].0, MODELS[7].1),
    mouse(MODELS[8].0, MODELS[8].1),
];

/// `sendColors`: each LED takes four bytes from offset 12 (R, G, B, unused).
pub fn colors(c: &[Color]) -> Vec<u8> {
    let mut b = COLOR_HEAD.to_vec();
    for &c in c {
        b.extend(rgb(c));
        b.push(0);
    }
    b
}

struct Glorious {
    wired: bool,
}

impl Proto for Glorious {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        io.feature(&colors(c), LEN)?;
        if self.wired {
            for p in LATCH {
                io.feature(&p, LEN)?;
            }
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        if self.wired {
            let _ = self.frame(io, &[Color(0, 0, 0); 2]);
            return;
        }
        let _ = io.feature(&HARDWARE, LEN);
        for p in LATCH {
            let _ = io.feature(&p, LEN);
        }
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let &(_, _, leds, wired) = MODELS
        .iter()
        .find(|m| m.0 == info.pid)
        .ok_or("unknown Glorious mouse")?;
    let pos: &[[u8; 2]] = if leds == 2 {
        &[[1, 1], [1, 0]]
    } else {
        &[[1, 1]]
    };
    let dev = placed(info, FAMILY, d.name, DeviceKind::Mouse, MOUSE, [3, 3], pos);
    boxed(api, info, "glorious wireless", dev, Glorious { wired })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::lights_b::rec::*;

    #[test]
    fn two_led_frame() {
        let b = colors(&[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(b[..12], COLOR_HEAD);
        assert_eq!(b[12..19], [1, 2, 3, 0, 4, 5, 6]);
    }

    #[test]
    fn wired_latches_every_frame() {
        let mut r = Rec::default();
        Glorious { wired: true }
            .frame(&mut r, &[Color(9, 9, 9)])
            .unwrap();
        let f = features(&r);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].len(), 65);
        assert_eq!(f[2][..9], LATCH[1]);
    }
}
