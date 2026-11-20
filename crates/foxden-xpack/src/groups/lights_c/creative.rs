//! Creative Sound BlasterX Katana (protocol learned from SignalRGB's Creative_Sound_BlasterX_Katana_Soundbar.js).

use super::common::{line, open_with};
use crate::hid::{Io, Proto};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "creative-katana";
const LEDS: usize = 7;
const LEN: usize = 65;
const HEADER: [u8; 9] = [0x00, 0x5A, 0x3A, 0x21, 0x0A, 0x00, 0x03, 0x01, 0x02];
const BRIGHTNESS: u8 = 0xFF;

/// One packet: header, then brightness and BGR per LED.
pub fn packet(colors: &[Color]) -> Vec<u8> {
    let mut p = HEADER.to_vec();
    for c in colors.iter().take(LEDS) {
        p.extend_from_slice(&[BRIGHTNESS, c.2, c.1, c.0]);
    }
    p
}

struct Katana;

impl Proto for Katana {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&packet(colors), LEN)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.write(&packet(&[Color(0, 0, 0); LEDS]), LEN);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(&info.device_id(FAMILY), d.name, DeviceKind::Other, LEDS);
    open_with(api, info, FAMILY, dev, Katana)
}

pub static DETECTORS: &[Detector] = &[crate::hid::on_i(
    "Creative Sound BlasterX Katana",
    0x041E,
    0x3247,
    4,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgr_after_brightness() {
        let p = packet(&[Color(1, 2, 3); LEDS]);
        assert_eq!(p.len(), 9 + 4 * LEDS);
        assert_eq!(p[..9], HEADER);
        assert_eq!(p[9..13], [0xFF, 3, 2, 1]);
    }
}
