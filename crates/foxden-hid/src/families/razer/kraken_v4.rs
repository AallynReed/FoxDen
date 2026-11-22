// SPDX-License-Identifier: GPL-2.0-or-later

use super::{model, stacked};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device};
use hidapi::{HidApi, HidDevice};

pub const REPORT_LEN: usize = 64;
const ARGS: usize = 10;
const KRAKEN_V4_WIRELESS: u16 = 0x056C;
const KRAKEN_KITTY_V3_PRO_WIRELESS: u16 = 0x0588;

pub type Report = [u8; REPORT_LEN];

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub tid: u8,
    pub wireless: u8,
    pub cols: usize,
}

impl Params {
    pub fn new(pid: u16) -> Params {
        let m = model(pid);
        Params {
            tid: m.tid,
            wireless: if matches!(pid, KRAKEN_V4_WIRELESS | KRAKEN_KITTY_V3_PRO_WIRELESS) {
                0x80
            } else {
                0x00
            },
            cols: m.cols as usize,
        }
    }
}

/// `razer_kraken_v4_create_report`. `razer_usb_send` leaves the CRC at zero.
fn report(p: &Params, class: u8, cmd: u8, size: u8) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[0] = 0x02;
    r[2] = p.tid;
    r[6] = size;
    r[7] = class;
    r[8] = cmd;
    r[9] = p.wireless;
    r
}

/// `SetDirect`.
pub fn set_direct(p: &Params, colors: &[Color]) -> Report {
    let n = p.cols.min(15);
    let mut r = report(p, 0x0F, 0x03, (5 + 3 * p.cols) as u8);
    r[ARGS + 4] = (p.cols as u8).wrapping_sub(1);
    for i in 0..n {
        let c = colors.get(i).copied().unwrap_or(Color::BLACK);
        r[ARGS + 5 + i * 3..ARGS + 8 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    r
}

pub struct KrakenV4 {
    hid: HidDevice,
    p: Params,
    device: Device,
    sent: Option<Vec<Color>>,
}

pub fn open(api: &HidApi, info: &Info, detector: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let m = model(info.pid);
    Ok(Box::new(KrakenV4 {
        hid,
        p: Params::new(info.pid),
        device: stacked(
            info.device_id("razer"),
            detector.name.into(),
            m.kind,
            m.zones,
        ),
        sent: None,
    }))
}

impl Backend for KrakenV4 {
    fn name(&self) -> &str {
        "razer kraken v4"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    /// Direct mode has no setup in `DeviceUpdateMode`; each push is one frame.
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        match self.hid.write(&set_direct(&self.p, colors)) {
            Ok(_) => {
                self.sent = Some(colors.to_vec());
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RazerKrakenV4Controller::SetDirect on the wired Kraken V4 (9 LEDs).
    #[test]
    fn wired_direct_frame() {
        let p = Params::new(0x056B);
        let colors: Vec<Color> = (0..9).map(|i| Color(i, i + 1, i + 2)).collect();
        let r = set_direct(&p, &colors);
        assert_eq!(
            r[..10],
            [0x02, 0x00, 0x60, 0x00, 0x00, 0x00, 32, 0x0F, 0x03, 0x00]
        );
        assert_eq!(r[10..15], [0, 0, 0, 0, 8]);
        for i in 0..9u8 {
            let at = 15 + i as usize * 3;
            assert_eq!(r[at..at + 3], [i, i + 1, i + 2]);
        }
        assert!(r[42..].iter().all(|&b| b == 0), "crc stays 0");
    }

    // RazerKrakenV4Controller constructor: wireless PIDs set the wireless flag.
    #[test]
    fn wireless_kitty_frame() {
        let p = Params::new(KRAKEN_KITTY_V3_PRO_WIRELESS);
        assert_eq!(p.cols, 10);
        let r = set_direct(&p, &[Color(9, 9, 9); 10]);
        assert_eq!(
            r[..10],
            [0x02, 0x00, 0x60, 0x00, 0x00, 0x00, 35, 0x0F, 0x03, 0x80]
        );
        assert_eq!(r[14], 9);
        assert_eq!(r[15 + 27..15 + 30], [9, 9, 9]);
        assert_eq!(r[45], 0);
        assert_eq!(Params::new(KRAKEN_V4_WIRELESS).wireless, 0x80);
    }
}
