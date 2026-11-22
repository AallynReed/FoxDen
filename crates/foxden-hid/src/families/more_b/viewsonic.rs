// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, at, backend, device, on_ip, on_ipu, open_io};
use crate::registry::{Detector, Info};

/// `VS_MODE_STATIC`: the monitors have no Direct mode, so OpenRGB drives their two LEDs per LED
/// through Static (QC) and Custom (QG). Both modes are AUTOMATIC_SAVE.
const MODE_STATIC: u8 = 0x01;
const QG_LEN: usize = 0x20;
const QC_LEN: usize = 167;

/// `SendModeComplete` of both controllers: base LEDs, then rear LEDs, then the end marker.
fn report(len: usize, base: Color, rear: Color) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[..16].copy_from_slice(&[
        0x02,
        MODE_STATIC,
        base.0,
        base.1,
        base.2,
        0x00,
        0x0A,
        0x00,
        MODE_STATIC,
        rear.0,
        rear.1,
        rear.2,
        0x00,
        0x0A,
        0x00,
        0x01,
    ]);
    b
}

struct Monitor {
    len: usize,
}

impl Driver for Monitor {
    // RGBController_XG270QG/QC::DeviceUpdateMode with the per-LED Static mode active.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.feature(&report(self.len, at(colors, 0), at(colors, 1)))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector, len: usize) -> Result<Box<dyn Backend>, String> {
    let io = open_io(api, info)?;
    let dev = device(
        info.device_id("viewsonic"),
        d.name,
        DeviceKind::Other,
        [1, 2],
        vec![[0.5, 0.85], [0.5, 0.25]],
    );
    Ok(backend(io, dev, Monitor { len }, "viewsonic monitor"))
}

fn open_qg(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, QG_LEN)
}

fn open_qc(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, QC_LEN)
}

pub(super) static DETECTORS: [Detector; 3] = [
    on_ipu(
        "ViewSonic Monitor XG270QG",
        0x0416,
        0x5020,
        0,
        0xFF00,
        1,
        open_qg,
    ),
    on_ip(
        "ViewSonic Monitor XG271QG",
        0x0543,
        0xA004,
        0,
        0x0001,
        open_qg,
    ),
    on_ipu(
        "ViewSonic Monitor XG270QC",
        0x0543,
        0xA002,
        0,
        0x0001,
        0,
        open_qc,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op, buf};
    use super::*;

    const HEAD: [u8; 16] = [
        0x02, 0x01, 1, 2, 3, 0x00, 0x0A, 0x00, 0x01, 4, 5, 6, 0x00, 0x0A, 0x00, 0x01,
    ];

    // VS_XG270QG_Controller::SendModeComplete(STATIC, base, STATIC, rear): 0x20 bytes
    #[test]
    fn qg_frame() {
        let mut fake = Fake::default();
        Monitor { len: QG_LEN }
            .frame(&mut fake, &[Color(1, 2, 3), Color(4, 5, 6)])
            .unwrap();
        assert_eq!(fake.ops(), vec![Op::Feature(buf(32, &HEAD))]);
    }

    // VS_XG270QC_Controller::SetMode(STATIC, base, STATIC, rear): 167 bytes
    #[test]
    fn qc_frame() {
        let mut fake = Fake::default();
        Monitor { len: QC_LEN }
            .frame(&mut fake, &[Color(1, 2, 3), Color(4, 5, 6)])
            .unwrap();
        assert_eq!(fake.ops(), vec![Op::Feature(buf(167, &HEAD))]);
    }
}
