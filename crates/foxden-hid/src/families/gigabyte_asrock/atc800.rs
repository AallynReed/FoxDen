//! Ported from OpenRGB Controllers/GigabyteAorusCPUCoolerController (GPL-2.0-or-later).

use super::{Driver, Op, Plan};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

pub static DETECTOR: Detector = Detector {
    name: "Gigabyte AORUS ATC800",
    vid: 0x1044,
    pid: 0x7A42,
    interface: Some(0),
    usage_page: Some(0xFF01),
    usage: Some(1),
    open,
};

const MODE_CUSTOM: u8 = 0x01;
const BRIGHTNESS_MAX: u8 = 0x05;

pub fn disable_temp_rpm_indicator() -> Vec<u8> {
    vec![0x00, 0xBE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xCC]
}

pub fn send_mode(mode: u8, brightness: u8, speed: u8, flag: u8, zone: u8) -> Vec<u8> {
    vec![0x00, 0xC9, mode, brightness, speed, flag, zone, 0x00, 0x00]
}

pub fn send_one_color(flag: u8, c: Color) -> Vec<u8> {
    vec![0x00, flag, c.0, c.1, c.2, 0x00, 0x00, 0x00, 0x00]
}

pub fn send_multi_color(flag: u8, mode: u8, a: Color, b: Color) -> Vec<u8> {
    vec![0x00, flag, mode, a.0, a.1, a.2, b.0, b.1, b.2]
}

pub fn send_ok() -> Vec<u8> {
    vec![0x00, 0xB6, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
}

/// SendCoolerMode for the Custom mode: zone 0 is the fan, zone 1 the top.
pub fn custom(zone: u8, c: Color) -> Vec<Vec<u8>> {
    let brightness = (BRIGHTNESS_MAX + 1) * 0x0A;
    let mut out = Vec::new();
    if zone == 1 {
        out.push(disable_temp_rpm_indicator());
        out.push(send_mode(MODE_CUSTOM, brightness, 0x02, 0x00, 0x00));
        out.push(send_one_color(0xBC, c));
    } else {
        out.push(send_mode(MODE_CUSTOM, brightness, 0x02, 0x00, 0x01));
        for flag in 0xB0..=0xB3 {
            out.push(send_multi_color(flag, MODE_CUSTOM, c, c));
        }
    }
    out.push(send_ok());
    out
}

pub struct Cooler;

impl Plan for Cooler {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        (0..2u8)
            .flat_map(|z| custom(z, colors[z as usize]))
            .map(Op::Feature)
            .collect()
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let mut device = Device::grid(
        &info.device_id("gigabyte-atc800"),
        d.name,
        DeviceKind::Other,
        [0.30, 0.30, 0.12, 0.12],
        1,
        2,
    );
    device.leds = vec![[0.5, 0.65], [0.5, 0.2]];
    Ok(Driver::boxed("gigabyte aorus atc800", device, hid, Cooler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    // RGBController_AorusATC800::DeviceUpdateLEDs -> ATC800Controller::SendCoolerMode(zone, CUSTOM).
    #[test]
    fn frame_sends_fan_then_top() {
        let mut s = Session::new(Cooler, 2);
        let fan = Color(0x10, 0x20, 0x30);
        let top = Color(0xA0, 0xB0, 0xC0);
        let ops = s.step(&[fan, top]);
        let expect: Vec<Vec<u8>> = vec![
            vec![0x00, 0xC9, 0x01, 0x3C, 0x02, 0x00, 0x01, 0x00, 0x00],
            vec![0x00, 0xB0, 0x01, 0x10, 0x20, 0x30, 0x10, 0x20, 0x30],
            vec![0x00, 0xB1, 0x01, 0x10, 0x20, 0x30, 0x10, 0x20, 0x30],
            vec![0x00, 0xB2, 0x01, 0x10, 0x20, 0x30, 0x10, 0x20, 0x30],
            vec![0x00, 0xB3, 0x01, 0x10, 0x20, 0x30, 0x10, 0x20, 0x30],
            vec![0x00, 0xB6, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            vec![0x00, 0xBE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xCC],
            vec![0x00, 0xC9, 0x01, 0x3C, 0x02, 0x00, 0x00, 0x00, 0x00],
            vec![0x00, 0xBC, 0xA0, 0xB0, 0xC0, 0x00, 0x00, 0x00, 0x00],
            vec![0x00, 0xB6, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ];
        assert_eq!(ops, expect.into_iter().map(Op::Feature).collect::<Vec<_>>());
        assert!(s.step(&[fan, top]).is_empty());
    }
}
