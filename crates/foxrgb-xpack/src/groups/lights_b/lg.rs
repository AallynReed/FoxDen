//! LG UltraGear 38GL950G monitor (protocol learned from SignalRGB's LG_UltraGear_38GL950G.js).

use super::layouts::LG38_POS;
use super::{MONITOR, boxed, det, placed};
use crate::hid::{Io, Proto};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "lg-ultragear";
const LEDS: usize = 48;
const LEN: usize = 65;
const ENABLE: [u8; 11] = [
    0x00, 0x53, 0x43, 0xCA, 0x02, 0x02, 0x03, 0x08, 0xD1, 0x45, 0x44,
];
const HEAD: [u8; 6] = [0x53, 0x43, 0xC1, 0x02, 0x91, 0x00];

pub static DETECTORS: &[Detector] = &[det(
    "LG UltraGear 38GL950G",
    0x043E,
    0x9A57,
    None,
    Some(0xFF00),
    Some(0x01),
    open,
)];

/// `sendColors`: C1 frame, zero channels raised to 1, XOR check, "ED", in three 64-byte reports.
pub fn frame(colors: &[Color]) -> [Vec<u8>; 3] {
    let mut data = HEAD.to_vec();
    for i in 0..LEDS {
        let c = colors.get(i).copied().unwrap_or(Color(0, 0, 0));
        data.extend([c.0, c.1, c.2].map(|v| v.max(1)));
    }
    let check = data.iter().fold(0u8, |a, b| a ^ b);
    data.extend([check, 0x45, 0x44]);
    data.resize(192, 0);
    let report = |i: usize| {
        let mut p = vec![0x00];
        p.extend_from_slice(&data[i * 64..(i + 1) * 64]);
        p
    };
    [report(0), report(1), report(2)]
}

struct Ultragear;

impl Proto for Ultragear {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&ENABLE, LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        frame(c).iter().try_for_each(|p| io.write(p, LEN))
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); LEDS]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        info,
        FAMILY,
        d.name,
        DeviceKind::Other,
        MONITOR,
        [16, 16],
        LG38_POS,
    );
    boxed(api, info, "lg ultragear", dev, Ultragear)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_and_checks() {
        let f = frame(&[Color(0, 0x10, 0); LEDS]);
        assert_eq!(
            f[0][1..10],
            [0x53, 0x43, 0xC1, 0x02, 0x91, 0x00, 1, 0x10, 1]
        );
        let tail = HEAD.len() + LEDS * 3 - 128;
        let check = HEAD.iter().fold(0u8, |a, b| a ^ b);
        assert_eq!(f[2][1 + tail..4 + tail], [check, 0x45, 0x44]);
        assert_eq!(LG38_POS.len(), LEDS);
    }
}
