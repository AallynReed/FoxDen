// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, at, device, on_ipu, open_with};
use crate::registry::{Detector, Info};

/// `NZXTMouseController::SendFirmwareRequest`.
fn firmware_request() -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x43;
    b[1] = 0x81;
    b[3] = 0x01;
    b
}

/// `NZXTMouseController::SetLEDs`: LED slots in the order 2, 1, 0, 3, 4, 5.
fn set_leds(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x43;
    b[1] = 0xAE;
    b[3] = 0x10;
    b[4] = 0x02;
    b[5] = 0x3F;
    b[24] = 0x06;
    for (slot, led) in [2, 1, 0, 3, 4, 5].into_iter().enumerate() {
        let c = at(colors, led);
        b[25 + slot * 4..28 + slot * 4].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

struct Lift;

impl Protocol for Lift {
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        vec![Op::Write(firmware_request()), Op::Read(64, 100)]
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        vec![Op::Write(set_leds(colors))]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "nzxt lift", |_| {
        let leds = [0.25, 0.75]
            .into_iter()
            .flat_map(|x| (0..3).map(move |i| [x, (i as f32 + 0.5) / 3.0]))
            .collect();
        let dev = device(
            info.device_id("nzxt-lift"),
            d.name,
            DeviceKind::Mouse,
            [2, 3],
            leds,
        );
        Ok((dev, Lift))
    })
}

pub(super) static DETECTORS: [Detector; 1] =
    [on_ipu("NZXT Lift", 0x1E71, 0x2100, 0, 0xFFCA, 1, open)];

#[cfg(test)]
mod tests {
    use super::super::test_util::buf;
    use super::*;

    // NZXTMouseController::SendFirmwareRequest
    #[test]
    fn setup_requests_firmware() {
        let want = buf(64, &[0x43, 0x81, 0x00, 0x01]);
        assert_eq!(Lift.setup(&[]), vec![Op::Write(want), Op::Read(64, 100)]);
    }

    // NZXTMouseController::SetLEDs
    #[test]
    fn frame_places_each_led() {
        let colors: Vec<Color> = (0..6).map(|i| Color(i, 10 + i, 20 + i)).collect();
        let mut want = buf(64, &[0x43, 0xAE, 0x00, 0x10, 0x02, 0x3F]);
        want[24] = 0x06;
        want[25..28].copy_from_slice(&[2, 12, 22]);
        want[29..32].copy_from_slice(&[1, 11, 21]);
        want[33..36].copy_from_slice(&[0, 10, 20]);
        want[37..40].copy_from_slice(&[3, 13, 23]);
        want[41..44].copy_from_slice(&[4, 14, 24]);
        want[45..48].copy_from_slice(&[5, 15, 25]);
        assert_eq!(Lift.frame(&colors), vec![Op::Write(want)]);
    }
}
