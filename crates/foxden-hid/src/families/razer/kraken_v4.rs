// SPDX-License-Identifier: GPL-2.0-or-later

use super::{model, stacked};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

pub const REPORT_LEN: usize = 64;
const ARGS: usize = 10;
const KRAKEN_V4_WIRELESS: u16 = 0x056C;
const KRAKEN_KITTY_V3_PRO_WIRELESS: u16 = 0x0588;
const KRAKEN_V4_PRO_HUB: u16 = 0x0568;
const KRAKEN_V4_PRO_CABLE: u16 = 0x0567;
/// The V4 Pro app repeats a steady frame this often.
const PRO_KEEPALIVE: Duration = Duration::from_secs(4);

pub type Report = [u8; REPORT_LEN];

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub tid: u8,
    pub wireless: u8,
    pub cols: usize,
    /// V4 Pro: checksummed reports, driver mode before frames.
    pub pro: bool,
}

impl Params {
    pub fn new(pid: u16) -> Params {
        let m = model(pid);
        Params {
            tid: m.tid,
            wireless: if matches!(
                pid,
                KRAKEN_V4_WIRELESS | KRAKEN_KITTY_V3_PRO_WIRELESS | KRAKEN_V4_PRO_HUB
            ) {
                0x80
            } else {
                0x00
            },
            cols: m.cols as usize,
            pro: matches!(pid, KRAKEN_V4_PRO_HUB | KRAKEN_V4_PRO_CABLE),
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

fn checksum(mut r: Report) -> Report {
    r[62] = r[..62].iter().fold(0, |a, b| a ^ b);
    r
}

/// `SetDirect`.
pub fn set_direct(p: &Params, colors: &[Color]) -> Report {
    let n = p.cols.min(15);
    let size = 5 + 3 * p.cols + p.pro as usize;
    let mut r = report(p, 0x0F, 0x03, size as u8);
    r[ARGS + 4] = (p.cols as u8).wrapping_sub(1);
    for i in 0..n {
        let c = colors.get(i).copied().unwrap_or(Color::BLACK);
        r[ARGS + 5 + i * 3..ARGS + 8 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    if p.pro { checksum(r) } else { r }
}

fn pro_mode(mode: u8) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[..11].copy_from_slice(&[0x02, 0, 0x60, 0, 0, 0, 0x02, 0x00, 0x04, 0x00, mode]);
    checksum(r)
}

fn pro_effect(p: &Params, effect: u8) -> Report {
    let mut r = report(p, 0x00, 0x00, 0x05);
    r[10..14].copy_from_slice(&[0xC0, 0x00, 0x01, effect]);
    checksum(r)
}

/// V4 Pro: take over lighting, driver mode, custom-frame effect.
pub fn pro_start(p: &Params) -> [Report; 3] {
    let mut take = report(p, 0x0F, 0x10, 0x02);
    take[10] = 0x01;
    [checksum(take), pro_mode(0x03), pro_effect(p, 0x08)]
}

/// V4 Pro: back to the default effect and normal mode.
pub fn pro_stop(p: &Params) -> [Report; 2] {
    [pro_effect(p, 0x0C), pro_mode(0x00)]
}

pub struct KrakenV4 {
    hid: HidDevice,
    p: Params,
    device: Device,
    sent: Option<Vec<Color>>,
    started: bool,
    last: Instant,
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
        started: false,
        last: Instant::now(),
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
        let stale = self.p.pro && self.last.elapsed() >= PRO_KEEPALIVE;
        if self.sent.as_deref() == Some(colors) && !stale {
            return Ok(());
        }
        if self.p.pro && !self.started {
            for r in pro_start(&self.p) {
                self.hid.write(&r).map_err(|e| e.to_string())?;
                std::thread::sleep(Duration::from_millis(20));
            }
            self.started = true;
        }
        match self.hid.write(&set_direct(&self.p, colors)) {
            Ok(_) => {
                self.sent = Some(colors.to_vec());
                self.last = Instant::now();
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e.to_string())
            }
        }
    }
}

impl Drop for KrakenV4 {
    fn drop(&mut self) {
        if self.started {
            for r in pro_stop(&self.p) {
                let _ = self.hid.write(&r);
                std::thread::sleep(Duration::from_millis(20));
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

    fn hex(s: &str) -> Vec<u8> {
        let mut v: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect();
        v.resize(REPORT_LEN, 0);
        v
    }

    fn with_crc(s: &str, crc: u8) -> Vec<u8> {
        let mut v = hex(s);
        v[62] = crc;
        v
    }

    // Kraken V4 Pro hub: a captured frame, byte for byte.
    #[test]
    fn pro_hub_frame_matches_capture() {
        let p = Params::new(KRAKEN_V4_PRO_HUB);
        let want = with_crc(
            "020060000000210f03800000000008ff6503e8ff0072ff0007ccff1664ff1858ff1858ff8739ffe8ff",
            0x2B,
        );
        let colors: Vec<Color> = want[15..42]
            .chunks(3)
            .map(|c| Color(c[0], c[1], c[2]))
            .collect();
        assert_eq!(set_direct(&p, &colors).to_vec(), want);
    }

    // Kraken V4 Pro hub: the start and stop sequences from the capture.
    #[test]
    fn pro_hub_start_and_stop_match_capture() {
        let p = Params::new(KRAKEN_V4_PRO_HUB);
        let start = pro_start(&p);
        assert_eq!(start[0].to_vec(), with_crc("020060000000020f108001", 0xFE));
        assert_eq!(start[1].to_vec(), with_crc("0200600000000200040003", 0x67));
        assert_eq!(
            start[2].to_vec(),
            with_crc("02006000000005000080c0000108", 0x2E)
        );
        let stop = pro_stop(&p);
        assert_eq!(
            stop[0].to_vec(),
            with_crc("02006000000005000080c000010c", 0x2A)
        );
        assert_eq!(stop[1].to_vec(), with_crc("0200600000000200040000", 0x64));
    }

    // On its own cable the headset takes the same packets with byte 9 cleared.
    #[test]
    fn pro_cable_clears_the_headset_flag() {
        let p = Params::new(KRAKEN_V4_PRO_CABLE);
        assert!(p.pro);
        assert_eq!(p.wireless, 0x00);
        let r = set_direct(&p, &[Color(1, 2, 3); 9]);
        assert_eq!(r[..10], [0x02, 0, 0x60, 0, 0, 0, 0x21, 0x0F, 0x03, 0x00]);
        assert_eq!(r[62], r[..62].iter().fold(0, |a, b| a ^ b));
    }
}
