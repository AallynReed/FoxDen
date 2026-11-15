//! Ported from OpenRGB Controllers/GigabyteAorusMouseController (GPL-2.0-or-later).

use super::{Driver, Op, Plan};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

pub static DETECTOR: Detector = Detector {
    name: "Gigabyte Aorus M2",
    vid: 0x1044,
    pid: 0x7A40,
    interface: Some(3),
    usage_page: Some(0xFF01),
    usage: Some(0x01),
    open,
};

const BRIGHTNESS_MAX: u8 = 0x64;

/// SetMode with the static value, which is how Direct sets brightness.
pub fn set_mode(c: Color, mode: u8, brightness: u8, speed: u8) -> Vec<u8> {
    vec![0xCC, mode, brightness, c.0, c.1, c.2, speed, 0x00]
}

pub fn send_direct(c: Color) -> Vec<u8> {
    vec![0xCD, 0x00, c.0, c.1, c.2, 0x00, 0x00, 0x00]
}

pub struct Mouse;

impl Plan for Mouse {
    fn first(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![Op::Feature(set_mode(colors[0], 0x01, BRIGHTNESS_MAX, 0))]
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![Op::Feature(send_direct(colors[0]))]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let device = Device::grid(
        &info.device_id("gigabyte-aorus-mouse"),
        d.name,
        DeviceKind::Mouse,
        [0.70, 0.62, 0.06, 0.22],
        1,
        1,
    );
    Ok(Driver::boxed("gigabyte aorus mouse", device, hid, Mouse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    // RGBController_GigabyteAorusMouse::DeviceUpdateMode (Direct) then DeviceUpdateLEDs / SendDirect.
    #[test]
    fn first_push_sets_brightness_then_sends_direct() {
        let mut s = Session::new(Mouse, 1);
        assert_eq!(
            s.step(&[Color(0x11, 0x22, 0x33)]),
            vec![
                Op::Feature(vec![0xCC, 0x01, 0x64, 0x11, 0x22, 0x33, 0x00, 0x00]),
                Op::Feature(vec![0xCD, 0x00, 0x11, 0x22, 0x33, 0x00, 0x00, 0x00]),
            ]
        );
        assert_eq!(
            s.step(&[Color(0xFF, 0x00, 0x80)]),
            vec![Op::Feature(vec![
                0xCD, 0x00, 0xFF, 0x00, 0x80, 0x00, 0x00, 0x00
            ])]
        );
    }
}
