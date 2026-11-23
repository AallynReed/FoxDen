// SPDX-License-Identifier: GPL-2.0-or-later

mod astro_a50;
mod g203l;
mod g213;
mod g600;
mod g815;
mod g9;
mod g915;
mod g933;
mod keys;
mod lightspeed;
mod lightsync;
mod x56;

use foxden_core::Color;
use hidapi::{HidApi, HidDevice};

use crate::generic_any::receiver::path_key;
use crate::registry::{Detector, Info, Open};

const VID: u16 = 0x046D;
const X56_VID: u16 = 0x0738;
const FAMILY: &str = "logitech";

/// What the drivers need from a HID handle, so tests can stand in for the device.
pub(crate) trait Hid: Send {
    fn write(&self, data: &[u8]) -> Result<usize, String>;
    /// Bytes read into `buf`, 0 on timeout.
    fn read(&self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String>;
    fn feature(&self, data: &[u8]) -> Result<(), String>;
}

impl Hid for HidDevice {
    fn write(&self, data: &[u8]) -> Result<usize, String> {
        HidDevice::write(self, data).map_err(|e| e.to_string())
    }

    fn read(&self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        self.read_timeout(buf, timeout_ms)
            .map_err(|e| e.to_string())
    }

    fn feature(&self, data: &[u8]) -> Result<(), String> {
        self.send_feature_report(data).map_err(|e| e.to_string())
    }
}

/// Writes a report and waits for the device's answer.
fn ack(hid: &dyn Hid, report: &[u8], timeout_ms: i32) -> Result<(), String> {
    hid.write(report)?;
    let mut buf = [0u8; 64];
    hid.read(&mut buf, timeout_ms)?;
    Ok(())
}

/// Colour `i` of a frame, black when the frame is short.
fn at(colors: &[Color], i: usize) -> Color {
    colors.get(i).copied().unwrap_or(Color::BLACK)
}

/// Opens the collection of the same physical interface whose usage page and usage match.
fn open_sibling(api: &HidApi, info: &Info, page: u16, usage: u16) -> Result<HidDevice, String> {
    let key = path_key(&info.path);
    let sibling = crate::device_list(api).iter()
        .map(Info::from_hidapi)
        .find(|d| {
            d.vid == info.vid
                && d.pid == info.pid
                && d.interface == info.interface
                && d.usage_page == page
                && d.usage == usage
                && path_key(&d.path) == key
        })
        .ok_or_else(|| format!("no collection {page:04x}:{usage:04x}"))?;
    sibling.open(api)
}

/// LED positions of a `rows` x `cols` matrix map, in LED order. `NA` cells are empty.
fn matrix_leds<const C: usize>(map: &[[u8; C]], count: usize) -> Vec<[f32; 2]> {
    let mut leds = vec![[0.5, 0.5]; count];
    for (r, row) in map.iter().enumerate() {
        for (c, &led) in row.iter().enumerate() {
            if (led as usize) < count {
                leds[led as usize] = [
                    (c as f32 + 0.5) / C as f32,
                    (r as f32 + 0.5) / map.len() as f32,
                ];
            }
        }
    }
    leds
}

const fn det(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: Option<i32>,
    usage_page: Option<u16>,
    usage: Option<u16>,
    open: Open,
) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface,
        usage_page,
        usage,
        open,
    }
}

const fn ipu(
    name: &'static str,
    vid: u16,
    pid: u16,
    i: i32,
    p: u16,
    u: u16,
    open: Open,
) -> Detector {
    det(name, vid, pid, Some(i), Some(p), Some(u), open)
}

const fn ip(name: &'static str, pid: u16, i: i32, p: u16, open: Open) -> Detector {
    det(name, VID, pid, Some(i), Some(p), None, open)
}

const fn pu(name: &'static str, pid: u16, p: u16, u: u16, open: Open) -> Detector {
    det(name, VID, pid, None, Some(p), Some(u), open)
}

static DETECTORS: [Detector; 41] = [
    ipu("Logitech G213", VID, 0xC336, 1, 0xFF43, 0x0602, g213::open),
    ip(
        "Logitech G813 RGB Mechanical Gaming Keyboard",
        0xC232,
        1,
        0xFF43,
        g815::open,
    ),
    ip(
        "Logitech G815 RGB Mechanical Gaming Keyboard",
        0xC33F,
        1,
        0xFF43,
        g815::open,
    ),
    ipu(
        "Logitech G915 Wireless RGB Mechanical Gaming Keyboard",
        VID,
        0xC541,
        2,
        0xFF00,
        2,
        g915::open,
    ),
    ipu(
        "Logitech G915 Wireless RGB Mechanical Gaming Keyboard (Wired)",
        VID,
        0xC33E,
        2,
        0xFF00,
        2,
        g915::open,
    ),
    ipu(
        "Logitech G915TKL Wireless RGB Mechanical Gaming Keyboard",
        VID,
        0xC545,
        2,
        0xFF00,
        2,
        g915::open,
    ),
    ipu(
        "Logitech G915TKL Wireless RGB Mechanical Gaming Keyboard (Wired)",
        VID,
        0xC343,
        2,
        0xFF00,
        2,
        g915::open,
    ),
    ip("Logitech G203 Prodigy", 0xC084, 1, 0xFF00, lightsync::open),
    ipu(
        "Logitech G203 Lightsync",
        VID,
        0xC092,
        1,
        0xFF00,
        2,
        g203l::open,
    ),
    ipu(
        "Logitech G203 Lightsync",
        VID,
        0xC09D,
        1,
        0xFF00,
        2,
        g203l::open,
    ),
    ip(
        "Logitech G303 Daedalus Apex",
        0xC080,
        1,
        0xFF00,
        lightsync::open,
    ),
    ip("Logitech G403 HERO", 0xC08F, 1, 0xFF00, lightsync::open),
    ip("Logitech G600 Gaming Mouse", 0xC24A, 1, 0xFF80, g600::open),
    ip(
        "Logitech G Pro Gaming Mouse",
        0xC085,
        1,
        0xFF00,
        lightsync::open,
    ),
    ip(
        "Logitech G Pro HERO Gaming Mouse",
        0xC08C,
        1,
        0xFF00,
        lightsync::open,
    ),
    ipu(
        "Logitech G933 Lightsync Headset",
        VID,
        0x0A5B,
        3,
        0xFF43,
        514,
        g933::open,
    ),
    ipu(
        "Logitech X56 Rhino Hotas Joystick",
        X56_VID,
        0x2221,
        2,
        0xFF00,
        1,
        x56::open,
    ),
    ipu(
        "Logitech X56 Rhino Hotas Throttle",
        X56_VID,
        0xA221,
        2,
        0xFF00,
        1,
        x56::open,
    ),
    ipu(
        "Logitech X56 Rhino Hotas Joystick",
        X56_VID,
        0x2221,
        2,
        0xFF00,
        3,
        x56::open,
    ),
    ipu(
        "Logitech X56 Rhino Hotas Throttle",
        X56_VID,
        0xA221,
        2,
        0xFF00,
        3,
        x56::open,
    ),
    ipu(
        "Logitech G Powerplay Mousepad",
        VID,
        0xC53A,
        2,
        0xFF00,
        1,
        lightspeed::open_receiver,
    ),
    ipu(
        "Logitech G502 Proteus Spectrum Gaming Mouse",
        VID,
        0xC332,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G502 HERO Gaming Mouse",
        VID,
        0xC08B,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G403 Prodigy Gaming Mouse",
        VID,
        0xC083,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G403 Wireless Gaming Mouse (wired)",
        VID,
        0xC082,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G703 Wireless Gaming Mouse (wired)",
        VID,
        0xC087,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    pu(
        "Logitech G703 HERO Wireless Gaming Mouse (wired)",
        0xC090,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G900 Wireless Gaming Mouse (wired)",
        VID,
        0xC081,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G903 Wireless Gaming Mouse (wired)",
        VID,
        0xC086,
        1,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G903 HERO Wireless Gaming Mouse (wired)",
        VID,
        0xC091,
        2,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G Pro Wireless Gaming Mouse (wired)",
        VID,
        0xC088,
        2,
        0xFF00,
        2,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G633 Gaming Headset",
        VID,
        0x0A5C,
        3,
        0xFF43,
        514,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G635 Gaming Headset",
        VID,
        0x0A89,
        3,
        0xFF43,
        514,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G733 Gaming Headset",
        VID,
        0x0AB5,
        3,
        0xFF43,
        514,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G733 Gaming Headset",
        VID,
        0x0AFE,
        3,
        0xFF43,
        514,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G733 Gaming Headset",
        VID,
        0x0B1F,
        3,
        0xFF43,
        514,
        lightspeed::open_wired,
    ),
    ipu(
        "Logitech G935 Gaming Headset",
        VID,
        0x0A87,
        3,
        0xFF43,
        514,
        lightspeed::open_wired,
    ),
    pu("Logitech G9", 0xC048, 0xFF00, 0x0001, g9::open),
    pu("Logitech G9x", 0xC066, 0xFF00, 0x0001, g9::open),
    pu(
        "Logitech G9x Call of Duty MW3 Edition",
        0xC249,
        0xFF00,
        0x0001,
        g9::open,
    ),
    pu(
        "Logitech ASTRO A50",
        0x0B1C,
        0xFF32,
        0x0074,
        astro_a50::open,
    ),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::Hid;

    pub type Log = Arc<Mutex<Vec<Vec<u8>>>>;
    type Reply = Box<dyn Fn(&[u8]) -> Vec<Vec<u8>> + Send>;

    /// Records every report and answers each write with whatever `reply` returns.
    pub struct Fake {
        pub log: Log,
        queue: Mutex<VecDeque<Vec<u8>>>,
        reply: Reply,
    }

    impl Fake {
        pub fn new(log: Log, reply: impl Fn(&[u8]) -> Vec<Vec<u8>> + Send + 'static) -> Self {
            Fake {
                log,
                queue: Mutex::new(VecDeque::new()),
                reply: Box::new(reply),
            }
        }

        /// A device that never answers.
        pub fn silent() -> (Self, Log) {
            let log = Log::default();
            (Fake::new(log.clone(), |_| Vec::new()), log)
        }
    }

    impl Hid for Fake {
        fn write(&self, data: &[u8]) -> Result<usize, String> {
            self.log.lock().unwrap().push(data.to_vec());
            self.queue.lock().unwrap().extend((self.reply)(data));
            Ok(data.len())
        }

        fn read(&self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            match self.queue.lock().unwrap().pop_front() {
                Some(r) => {
                    let n = r.len().min(buf.len());
                    buf[..n].copy_from_slice(&r[..n]);
                    Ok(n)
                }
                None => Ok(0),
            }
        }

        fn feature(&self, data: &[u8]) -> Result<(), String> {
            self.log.lock().unwrap().push(data.to_vec());
            Ok(())
        }
    }

    pub fn take(log: &Log) -> Vec<Vec<u8>> {
        std::mem::take(&mut *log.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known detector ids this folder covers.
    #[test]
    fn detector_lines() {
        let d = detectors();
        assert_eq!(d.len(), 37 + 4);
        assert!(d.iter().all(|d| !crate::hidpp::PIDS.contains(&d.pid)));
        let g502 = d.iter().find(|d| d.pid == 0xC090).unwrap();
        assert_eq!(
            (g502.interface, g502.usage_page, g502.usage),
            (None, Some(0xFF00), Some(2))
        );
        let g815 = d.iter().find(|d| d.pid == 0xC33F).unwrap();
        assert_eq!(
            (g815.interface, g815.usage_page, g815.usage),
            (Some(1), Some(0xFF43), None)
        );
        let x56: Vec<_> = d.iter().filter(|d| d.vid == X56_VID).collect();
        assert_eq!(x56.len(), 4);
        let g933 = d.iter().find(|d| d.pid == 0x0A5B).unwrap();
        assert_eq!(g933.usage, Some(0x0202));
    }
}
