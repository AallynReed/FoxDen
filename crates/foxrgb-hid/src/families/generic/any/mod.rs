//! Ported from OpenRGB Controllers/HIDLampArrayController and Controllers/LogitechController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::DeviceKind;
use hidapi::HidDevice;

use crate::registry::{Bus, Matcher};

mod hidpp20;
mod lamparray;
mod receiver;

/// `REGISTER_HID_DETECTOR_PU_ONLY` and the unified Logitech HID++ 2.0 lines. The Centurion
/// line (usage page 0xFFA0) is left out: its transport is not ported.
pub fn matchers() -> Vec<&'static Matcher> {
    hidpp20::MATCHERS
        .iter()
        .chain(&lamparray::MATCHERS)
        .collect()
}

/// What a driver does with an open collection, so tests can stand in for the device.
pub(crate) trait Link: Send {
    fn write(&mut self, data: &[u8]) -> Result<usize, String>;
    /// Bytes read, 0 on timeout.
    fn read(&mut self, buf: &mut [u8], ms: i32) -> Result<usize, String>;
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String>;
    /// `buf[0]` holds the report id.
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    fn descriptor(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    fn sleep(&mut self, ms: u64);
}

impl Link for HidDevice {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        HidDevice::write(self, data).map_err(|e| e.to_string())
    }

    fn read(&mut self, buf: &mut [u8], ms: i32) -> Result<usize, String> {
        self.read_timeout(buf, ms).map_err(|e| e.to_string())
    }

    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        self.send_feature_report(data).map_err(|e| e.to_string())
    }

    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.get_feature_report(buf).map_err(|e| e.to_string())
    }

    fn descriptor(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.get_report_descriptor(buf).map_err(|e| e.to_string())
    }

    fn sleep(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Where each kind of device sits on the canvas by default.
fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        _ => [0.34, 0.12, 0.24, 0.24],
    }
}

fn on_usb_or_bluetooth(bus: Bus) -> bool {
    matches!(bus, Bus::Usb | Bus::Bluetooth)
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::{HashMap, VecDeque};

    use super::Link;

    /// One thing the driver did to the device.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Ev {
        Write(Vec<u8>),
        SendFeature(Vec<u8>),
        GetFeature(u8, usize),
        Sleep(u64),
    }

    type Reply = Box<dyn FnMut(&[u8]) -> Vec<Vec<u8>> + Send>;

    /// Records every operation; `reply` turns each write into the input reports that follow it.
    pub struct Fake {
        pub log: Vec<Ev>,
        pub input: VecDeque<Vec<u8>>,
        pub features: HashMap<u8, VecDeque<Vec<u8>>>,
        pub desc: Vec<u8>,
        pub reply: Reply,
    }

    impl Fake {
        pub fn new(reply: impl FnMut(&[u8]) -> Vec<Vec<u8>> + Send + 'static) -> Fake {
            Fake {
                log: Vec::new(),
                input: VecDeque::new(),
                features: HashMap::new(),
                desc: Vec::new(),
                reply: Box::new(reply),
            }
        }

        pub fn writes(&self) -> Vec<Vec<u8>> {
            self.log
                .iter()
                .filter_map(|e| match e {
                    Ev::Write(b) => Some(b.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl Link for Fake {
        fn write(&mut self, data: &[u8]) -> Result<usize, String> {
            self.log.push(Ev::Write(data.to_vec()));
            let out = (self.reply)(data);
            self.input.extend(out);
            Ok(data.len())
        }

        fn read(&mut self, buf: &mut [u8], _ms: i32) -> Result<usize, String> {
            match self.input.pop_front() {
                Some(r) => {
                    let n = r.len().min(buf.len());
                    buf[..n].copy_from_slice(&r[..n]);
                    Ok(n)
                }
                None => Ok(0),
            }
        }

        fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
            self.log.push(Ev::SendFeature(data.to_vec()));
            Ok(())
        }

        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            self.log.push(Ev::GetFeature(buf[0], buf.len()));
            let r = self
                .features
                .get_mut(&buf[0])
                .and_then(|q| q.pop_front())
                .ok_or("no report")?;
            let n = r.len().min(buf.len());
            buf[..n].copy_from_slice(&r[..n]);
            Ok(n)
        }

        fn descriptor(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            let n = self.desc.len().min(buf.len());
            buf[..n].copy_from_slice(&self.desc[..n]);
            Ok(n)
        }

        fn sleep(&mut self, ms: u64) {
            self.log.push(Ev::Sleep(ms));
        }
    }
}
