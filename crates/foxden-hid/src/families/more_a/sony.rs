// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const VID: u16 = 0x054C;
const FAMILY: &str = "sony";

/// CRCPP CRC_32: reflected 0x04C11DB7, init and final xor 0xFFFFFFFF.
pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(0xFFFF_FFFFu32, |mut crc, &b| {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
        crc
    })
}

/// Drops the 0xA2 the report is signed with and puts the CRC of the first 75 bytes at the end.
fn signed_bt(buffer: &[u8; 79]) -> Vec<u8> {
    let crc = crc32(&buffer[..75]);
    let mut out = buffer[1..].to_vec();
    out[74..78].copy_from_slice(&crc.to_le_bytes());
    out
}

/// SonyDS4Controller constructor: feature report 0x02, then up to five reads looking for report 17.
fn ds4_is_bluetooth(io: &mut dyn Io) -> Result<bool, String> {
    let mut report = [0u8; 64];
    report[0] = 0x02;
    io.get_feature(&mut report)?;
    let mut read = [0u8; 64];
    for _ in 0..5 {
        io.read(&mut read, BLOCKING_MS)?;
        if read[0] == 17 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// SonyDS4Controller::sendReportBT / sendReportUSB.
fn ds4_report(bluetooth: bool, c: Color) -> Vec<u8> {
    if bluetooth {
        let mut b = [0u8; 79];
        b[..5].copy_from_slice(&[0xA2, 0x11, 0xC0, 0x00, 0x07]);
        b[9..12].copy_from_slice(&rgb(c));
        signed_bt(&b)
    } else {
        vec![0x05, 0x07, 0, 0, 0, 0, c.0, c.1, c.2, 0, 0]
    }
}

struct Ds4 {
    bluetooth: bool,
}

impl Proto for Ds4 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&ds4_report(self.bluetooth, colors[0]))
    }
}

const DIRECT_MODE: u8 = 0x01;
const DEFAULT_BRIGHTNESS: u8 = 0x01;

/// SonyDualSenseController::SetColors with Direct mode at its default brightness.
fn dualsense_report(bluetooth: bool, colors: &[Color]) -> Vec<u8> {
    let lit = |i: usize| (colors[i] != Color::BLACK) as u8;
    let players = 0x20 + lit(1) + (lit(2) << 1) + (lit(3) << 2) + (lit(4) << 3) + (lit(5) << 4);
    if bluetooth {
        let mut b = [0u8; 79];
        b[..5].copy_from_slice(&[0xA2, 0x31, 0x02, 0x0F, 0x55]);
        b[11] = DIRECT_MODE;
        b[41] = 0xFF;
        b[44] = 0x02;
        b[45] = 0x02 - DEFAULT_BRIGHTNESS;
        b[46] = players;
        b[47..50].copy_from_slice(&rgb(colors[0]));
        signed_bt(&b)
    } else {
        let mut b = vec![0u8; 48];
        b[..3].copy_from_slice(&[0x02, 0x0F, 0x55]);
        b[9] = DIRECT_MODE;
        b[39] = 0xFF;
        b[43] = 0x02 - DEFAULT_BRIGHTNESS;
        b[44] = players;
        b[45..48].copy_from_slice(&rgb(colors[0]));
        b
    }
}

struct DualSense {
    bluetooth: bool,
}

impl Proto for DualSense {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&dualsense_report(self.bluetooth, colors))
    }
}

fn open_ds4(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let bluetooth = ds4_is_bluetooth(io.as_mut())?;
    let dev = line(info, FAMILY, d, DeviceKind::Other, 1);
    Ok(boxed(io, FAMILY, dev, Ds4 { bluetooth }))
}

fn open_dualsense(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = opened(api, info)?;
    let bluetooth = info.interface == -1;
    let mut leds = vec![[0.5, 0.25]];
    leds.extend((0..5).map(|i| [(i as f32 + 0.5) / 5.0, 0.75]));
    let mut dev = placed(info, FAMILY, d, DeviceKind::Other, [5, 2], leds);
    if bluetooth {
        dev.name.push_str(" (BT)");
    }
    Ok(boxed(io, FAMILY, dev, DualSense { bluetooth }))
}

pub static DETECTORS: [Detector; 5] = [
    det("Sony DualShock 4", VID, 0x05C4, ANY, ANY, ANY, open_ds4),
    det("Sony DualShock 4", VID, 0x09CC, ANY, ANY, ANY, open_ds4),
    det("Sony DualShock 4", VID, 0x0BA0, ANY, ANY, ANY, open_ds4),
    det("Sony DualSense", VID, 0x0CE6, ANY, ANY, ANY, open_dualsense),
    det(
        "Sony DualSense Edge",
        VID,
        0x0DF2,
        ANY,
        ANY,
        ANY,
        open_dualsense,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // CRCPP::CRC::CRC_32 check value
    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    // SonyDS4Controller constructor: report 17 on a read means Bluetooth
    #[test]
    fn ds4_detects_bluetooth() {
        let mut usb = Rec::with(vec![vec![0x02; 64], vec![0x01; 64], vec![0x01; 64]]);
        assert!(!ds4_is_bluetooth(&mut usb).unwrap());
        assert_eq!(
            usb.take(),
            [vec![Op::GetF(0x02, 64)], vec![Op::Read(64); 5]].concat()
        );
        let mut bt = Rec::with(vec![vec![0x02; 64], vec![0x01; 64], vec![0x11; 64]]);
        assert!(ds4_is_bluetooth(&mut bt).unwrap());
        assert_eq!(bt.take().len(), 3);
    }

    // SonyDS4Controller::sendReportUSB
    #[test]
    fn ds4_usb_frame() {
        let [first, again, next] = pushes(
            Ds4 { bluetooth: false },
            1,
            vec![],
            &[Color(1, 2, 3)],
            &[Color(4, 5, 6)],
        );
        assert_eq!(first, vec![w(vec![5, 7, 0, 0, 0, 0, 1, 2, 3, 0, 0])]);
        assert!(again.is_empty());
        assert_eq!(next, vec![w(vec![5, 7, 0, 0, 0, 0, 4, 5, 6, 0, 0])]);
    }

    // SonyDS4Controller::sendReportBT
    #[test]
    fn ds4_bluetooth_frame() {
        let r = ds4_report(true, Color(0x10, 0x20, 0x30));
        assert_eq!(r.len(), 78);
        assert_eq!(
            &r[..11],
            &[0x11, 0xC0, 0, 0x07, 0, 0, 0, 0, 0x10, 0x20, 0x30]
        );
        assert!(r[11..74].iter().all(|&b| b == 0));
        let mut signed = vec![0xA2];
        signed.extend_from_slice(&r[..74]);
        assert_eq!(r[74..], crc32(&signed).to_le_bytes());
    }

    // SonyDualSenseController::SetColors over USB
    #[test]
    fn dualsense_usb_frame() {
        let mut c = vec![Color::BLACK; 6];
        c[0] = Color(9, 8, 7);
        c[2] = Color(0, 0, 1);
        c[5] = Color(1, 0, 0);
        let r = dualsense_report(false, &c);
        let mut want = pad(&[0x02, 0x0F, 0x55], 48);
        want[9] = 0x01;
        want[39] = 0xFF;
        want[43] = 0x01;
        want[44] = 0x20 | 0x02 | 0x10;
        want[45..48].copy_from_slice(&[9, 8, 7]);
        assert_eq!(r, want);
    }

    // SonyDualSenseController::SetColors over Bluetooth
    #[test]
    fn dualsense_bluetooth_frame() {
        let mut c = vec![Color::BLACK; 6];
        c[0] = Color(1, 2, 3);
        c[1] = Color(5, 5, 5);
        let r = dualsense_report(true, &c);
        let mut b = [0u8; 79];
        b[..5].copy_from_slice(&[0xA2, 0x31, 0x02, 0x0F, 0x55]);
        b[11] = 1;
        b[41] = 0xFF;
        b[44] = 2;
        b[45] = 1;
        b[46] = 0x21;
        b[47..50].copy_from_slice(&[1, 2, 3]);
        let crc = crc32(&b[..75]);
        let mut want = b[1..].to_vec();
        want[74..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(r, want);
        assert_eq!(r.len(), 78);
    }
}
