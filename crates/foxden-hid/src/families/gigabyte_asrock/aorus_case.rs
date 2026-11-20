//! Ported from OpenRGB Controllers/GigabyteAorusPCCaseController (GPL-2.0-or-later).

use super::{Driver, Op, Plan};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

pub static DETECTOR: Detector = Detector {
    name: "Gigabyte AORUS C300 GLASS",
    vid: 0x1044,
    pid: 0x7A30,
    interface: Some(0),
    usage_page: Some(0xFF01),
    usage: Some(0x01),
    open,
};

const MODE_CUSTOM: u8 = 0x01;
const SPEED_NORMAL: u8 = 0x09;
const BRIGHTNESS_MAX: u8 = 0x09;

pub fn send_color(c: Color) -> Vec<u8> {
    vec![0x00, 0x01, 0xC8, c.0, c.1, c.2, 0x08, 0x01, 0x00]
}

pub fn send_mode(mode: u8, speed: u8, brightness: u8) -> Vec<u8> {
    vec![0x00, 0x01, 0xC9, mode, brightness, speed, 0x01, 0x08, 0x00]
}

pub fn send_ok() -> Vec<u8> {
    vec![0x00, 0x01, 0xB6, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
}

/// The case has no direct mode; Custom is its single-colour mode.
pub struct Case;

impl Plan for Case {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![
            Op::Feature(send_color(colors[0])),
            Op::Feature(send_mode(MODE_CUSTOM, SPEED_NORMAL, BRIGHTNESS_MAX)),
            Op::Feature(send_ok()),
        ]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let device = Device::grid(
        &info.device_id("gigabyte-aorus-case"),
        d.name,
        DeviceKind::Other,
        [0.84, 0.10, 0.10, 0.36],
        1,
        1,
    );
    Ok(Driver::boxed("gigabyte aorus case", device, hid, Case))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    // GigabyteAorusPCCaseController::SetMode(AORUS_PC_CASE_MODE_CUSTOM, ...) at full brightness.
    #[test]
    fn custom_frame_matches_set_mode() {
        let mut s = Session::new(Case, 1);
        assert_eq!(
            s.step(&[Color(0x12, 0x34, 0x56)]),
            vec![
                Op::Feature(vec![0x00, 0x01, 0xC8, 0x12, 0x34, 0x56, 0x08, 0x01, 0x00]),
                Op::Feature(vec![0x00, 0x01, 0xC9, 0x01, 0x09, 0x09, 0x01, 0x08, 0x00]),
                Op::Feature(vec![0x00, 0x01, 0xB6, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ]
        );
    }

    // SendMode puts brightness before speed.
    #[test]
    fn send_mode_byte_order() {
        assert_eq!(
            send_mode(0x02, 0x0A, 0x05),
            vec![0x00, 0x01, 0xC9, 0x02, 0x05, 0x0A, 0x01, 0x08, 0x00]
        );
    }
}
