// SPDX-License-Identifier: GPL-2.0-or-later

use super::{Driver, Op, Plan, run};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

pub static DETECTOR: Detector = Detector {
    name: "Gigabyte Aorus Waterforce X II 360",
    vid: 0x0414,
    pid: 0x7A5E,
    interface: Some(0),
    usage_page: Some(0x0000),
    usage: Some(0x0002),
    open,
};

const REPORT_ID: u8 = 0x99;
const OUT_TOTAL: usize = 6144;
const IN_SIZE: usize = 255;
const STYLE_STATIC: u8 = 0x01;
const SPEED_DEFAULT: u8 = 5;
const BRIGHTNESS_DEFAULT: u8 = 10;

/// SendPacket: report 0x99, payload, zero padding to 6144 bytes.
pub fn packet(payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; OUT_TOTAL];
    b[0] = REPORT_ID;
    let n = payload.len().min(OUT_TOTAL - 1);
    b[1..1 + n].copy_from_slice(&payload[..n]);
    b
}

/// SetEffect for Static: no speed flag, so speed stays at the default.
pub fn static_effect(c: Color) -> Vec<Vec<u8>> {
    vec![
        packet(&[
            0xC9,
            STYLE_STATIC,
            SPEED_DEFAULT * 20,
            BRIGHTNESS_DEFAULT,
            0x02,
            0x01,
        ]),
        packet(&[0xCD, c.0, c.1, c.2]),
        packet(&[0xB6]),
    ]
}

/// The cooler has no direct mode; Static is its single-colour mode.
pub struct Cooler;

impl Plan for Cooler {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        static_effect(colors[0])
            .into_iter()
            .map(Op::Write)
            .collect()
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.port(api)?;
    // Firmware query from the constructor; the answer only names the version.
    let mut answer = [0u8; IN_SIZE];
    run(&hid, &[Op::Write(packet(&[0xDE, 0x00]))])?;
    let _ = hid.read_timeout(&mut answer, 2000);
    let device = Device::grid(
        &info.device_id("gigabyte-castor3"),
        d.name,
        DeviceKind::Other,
        [0.30, 0.30, 0.12, 0.12],
        1,
        1,
    );
    Ok(Driver::boxed("gigabyte castor3", device, hid, Cooler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    fn padded(head: &[u8]) -> Vec<u8> {
        let mut v = head.to_vec();
        v.resize(6144, 0);
        v
    }

    // RGBController_GigabyteCastor3::DeviceUpdateMode (Static) -> SetEffect -> SendPacket.
    #[test]
    fn static_frame_matches_set_effect() {
        let mut s = Session::new(Cooler, 1);
        let ops = s.step(&[Color(0x01, 0x80, 0xFE)]);
        assert_eq!(
            ops,
            vec![
                Op::Write(padded(&[0x99, 0xC9, 0x01, 0x64, 0x0A, 0x02, 0x01])),
                Op::Write(padded(&[0x99, 0xCD, 0x01, 0x80, 0xFE])),
                Op::Write(padded(&[0x99, 0xB6])),
            ]
        );
    }

    // GigabyteCastor3Controller constructor: de 00 firmware query.
    #[test]
    fn firmware_query_packet() {
        assert_eq!(packet(&[0xDE, 0x00]), padded(&[0x99, 0xDE, 0x00]));
    }
}
