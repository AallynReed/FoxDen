//! Ported from OpenRGB Controllers/ (shared HID plumbing for the mixed_a drivers) (GPL-2.0-or-later).

use std::time::{Duration, Instant};

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use crate::port::Port;
use crate::registry::Info;

/// The HID calls the drivers make, so tests can record them.
pub trait Io: Send {
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String>;
    /// `buf[0]` holds the report id; returns the length read.
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String>;
    fn sleep(&mut self, ms: u64);
}

impl Io for HidDevice {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        HidDevice::write(self, data)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        self.send_feature_report(data).map_err(|e| e.to_string())
    }

    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.get_feature_report(buf).map_err(|e| e.to_string())
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        self.read_timeout(buf, timeout_ms)
            .map_err(|e| e.to_string())
    }

    fn sleep(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

impl Io for Port {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        Port::write(self, data)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        self.send_feature_report(data).map_err(|e| e.to_string())
    }

    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.get_feature_report(buf).map_err(|e| e.to_string())
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        self.read_timeout(buf, timeout_ms)
            .map_err(|e| e.to_string())
    }

    fn sleep(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// One device's protocol: direct mode setup and one colour frame.
pub trait Driver: Send {
    fn start(&mut self, _io: &mut dyn Io, _colors: &[Color]) -> Result<(), String> {
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String>;
    fn stop(&mut self, _io: &mut dyn Io) {}
    /// Pushes sooner than this after the last frame are dropped, except all black.
    fn min_interval(&self) -> Duration {
        Duration::ZERO
    }
    /// An unchanged frame is sent again once this has passed.
    fn keepalive(&self) -> Option<Duration> {
        None
    }
}

/// A backend for one physical device driven by `D`.
pub struct Hid<D: Driver> {
    name: &'static str,
    io: Box<dyn Io>,
    drv: D,
    device: Device,
    started: bool,
    sent: Option<Vec<Color>>,
    last: Option<Instant>,
}

impl<D: Driver> Hid<D> {
    pub fn new(name: &'static str, io: Box<dyn Io>, drv: D, device: Device) -> Self {
        Hid {
            name,
            io,
            drv,
            device,
            started: false,
            sent: None,
            last: None,
        }
    }

    pub fn boxed(
        name: &'static str,
        io: impl Io + 'static,
        drv: D,
        device: Device,
    ) -> Box<dyn Backend>
    where
        D: 'static,
    {
        Box::new(Self::new(name, Box::new(io), drv, device))
    }
}

impl<D: Driver> Backend for Hid<D> {
    fn name(&self) -> &str {
        self.name
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let mut colors = colors.to_vec();
        colors.resize(self.device.leds.len(), Color::BLACK);
        let now = Instant::now();
        let since = self.last.map(|t| now.duration_since(t));
        let stale = match (self.drv.keepalive(), since) {
            (Some(k), Some(s)) => s >= k,
            _ => false,
        };
        if self.sent.as_deref() == Some(&colors[..]) && !stale {
            return Ok(());
        }
        let black = colors.iter().all(|c| *c == Color::BLACK);
        if since.is_some_and(|s| s < self.drv.min_interval()) && !black {
            return Ok(());
        }
        if !self.started {
            self.drv.start(self.io.as_mut(), &colors)?;
            self.started = true;
        }
        match self.drv.frame(self.io.as_mut(), &colors) {
            Ok(()) => {
                self.sent = Some(colors);
                self.last = Some(now);
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

impl<D: Driver> Drop for Hid<D> {
    fn drop(&mut self) {
        if self.started {
            self.drv.stop(self.io.as_mut());
        }
    }
}

/// Opens another collection of the same device, picked by `pick`.
pub fn open_sibling(
    api: &HidApi,
    info: &Info,
    pick: impl Fn(&Info) -> bool,
) -> Result<HidDevice, String> {
    crate::device_list(api).iter()
        .map(Info::from_hidapi)
        .filter(|s| s.vid == info.vid && s.pid == info.pid && s.serial == info.serial)
        .find(|s| pick(s))
        .ok_or_else(|| "sibling collection not found".to_string())?
        .open(api)
}

/// Every collection of the same device that `pick` accepts.
pub fn siblings(api: &HidApi, info: &Info, pick: impl Fn(&Info) -> bool) -> Vec<Info> {
    crate::device_list(api).iter()
        .map(Info::from_hidapi)
        .filter(|s| s.vid == info.vid && s.pid == info.pid && s.serial == info.serial)
        .filter(|s| pick(s))
        .collect()
}

pub fn line(id: String, name: &str, kind: DeviceKind, rect: [f32; 4], n: usize) -> Device {
    Device::grid(&id, name, kind, rect, n as u32, 1)
}

pub fn rows(
    id: String,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    cols: usize,
    rows: usize,
) -> Device {
    Device::grid(&id, name, kind, rect, cols as u32, rows as u32)
}

/// LEDs placed by a row-major matrix of LED indices; `NA` cells are empty and LEDs not in it go below.
pub fn matrix(
    id: String,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    (height, width): (usize, usize),
    map: &[u32],
    count: usize,
) -> Device {
    let mut pos: Vec<Option<[f32; 2]>> = vec![None; count];
    for (cell, &led) in map.iter().enumerate().take(height * width) {
        if let Some(p) = pos.get_mut(led as usize) {
            let (r, c) = (cell / width, cell % width);
            *p = Some([
                (c as f32 + 0.5) / width as f32,
                (r as f32 + 0.5) / (height + 1) as f32,
            ]);
        }
    }
    let loose = pos.iter().filter(|p| p.is_none()).count().max(1);
    let mut k = 0;
    let leds = pos
        .into_iter()
        .map(|p| {
            p.unwrap_or_else(|| {
                k += 1;
                [
                    (k as f32 - 0.5) / loose as f32,
                    (height as f32 + 0.5) / (height + 1) as f32,
                ]
            })
        })
        .collect();
    Device {
        id,
        name: name.into(),
        kind,
        rect,
        grid: [width as u32, height as u32 + 1],
        leds,
    }
}

pub const NA: u32 = 0xFFFF_FFFF;

pub const MOUSE: [f32; 4] = [0.70, 0.62, 0.06, 0.22];
pub const KEYBOARD: [f32; 4] = [0.06, 0.58, 0.54, 0.32];
pub const PAD: [f32; 4] = [0.62, 0.56, 0.32, 0.36];
pub const HEADSET: [f32; 4] = [0.84, 0.10, 0.08, 0.10];
pub const BOARD: [f32; 4] = [0.06, 0.12, 0.52, 0.05];
pub const FANS: [f32; 4] = [0.06, 0.24, 0.52, 0.24];

pub fn sum16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |s, &b| s.wrapping_add(b as u16))
}

#[cfg(test)]
pub mod mock {
    use super::Io;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Op {
        Write(Vec<u8>),
        Feature(Vec<u8>),
        GetFeature(Vec<u8>),
        Read(usize, i32),
        Sleep(u64),
    }

    /// Records every call; `get_feature` and `read` answer from `replies` in order.
    #[derive(Clone, Default)]
    pub struct Mock {
        pub log: Arc<Mutex<Vec<Op>>>,
        pub replies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }

    impl Mock {
        pub fn reply(&self, r: Vec<u8>) {
            self.replies.lock().unwrap().push_back(r);
        }

        pub fn take(&self) -> Vec<Op> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }

        /// An empty reply stands for a failed call.
        fn answer(&self, buf: &mut [u8]) -> Option<Result<usize, String>> {
            let r = self.replies.lock().unwrap().pop_front()?;
            if r.is_empty() {
                return Some(Err("mock failure".into()));
            }
            let n = r.len().min(buf.len());
            buf[..n].copy_from_slice(&r[..n]);
            Some(Ok(n))
        }
    }

    impl Io for Mock {
        fn write(&mut self, data: &[u8]) -> Result<(), String> {
            self.log.lock().unwrap().push(Op::Write(data.to_vec()));
            Ok(())
        }

        fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
            self.log.lock().unwrap().push(Op::Feature(data.to_vec()));
            Ok(())
        }

        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            self.log.lock().unwrap().push(Op::GetFeature(buf.to_vec()));
            self.answer(buf).unwrap_or(Ok(buf.len()))
        }

        fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
            self.log
                .lock()
                .unwrap()
                .push(Op::Read(buf.len(), timeout_ms));
            self.answer(buf).unwrap_or(Ok(0))
        }

        fn sleep(&mut self, ms: u64) {
            self.log.lock().unwrap().push(Op::Sleep(ms));
        }
    }

    /// `n` bytes starting with `head`, the rest zero.
    pub fn pad(head: &[u8], n: usize) -> Vec<u8> {
        let mut v = head.to_vec();
        v.resize(n, 0);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{Mock, Op};
    use super::*;

    struct Echo;

    impl Driver for Echo {
        fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
            io.send_feature(&[1])
        }
        fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
            io.write(&[colors[0].0])
        }
        fn stop(&mut self, io: &mut dyn Io) {
            let _ = io.send_feature(&[2]);
        }
    }

    #[test]
    fn setup_once_and_skip_unchanged_frames() {
        let m = Mock::default();
        let dev = line("x".into(), "X", DeviceKind::Other, MOUSE, 2);
        let mut b = Hid::new("t", Box::new(m.clone()), Echo, dev);
        b.push("x", &[Color(5, 0, 0)]).unwrap();
        b.push("x", &[Color(5, 0, 0), Color::BLACK]).unwrap();
        b.push("x", &[Color(6, 0, 0)]).unwrap();
        assert!(b.push("y", &[]).is_err());
        drop(b);
        assert_eq!(
            m.take(),
            vec![
                Op::Feature(vec![1]),
                Op::Write(vec![5]),
                Op::Write(vec![6]),
                Op::Feature(vec![2])
            ]
        );
    }

    #[test]
    fn matrix_places_listed_leds_and_puts_the_rest_below() {
        let d = matrix(
            "m".into(),
            "M",
            DeviceKind::Keyboard,
            KEYBOARD,
            (1, 2),
            &[1, NA],
            3,
        );
        assert_eq!(d.leds.len(), 3);
        assert_eq!(d.leds[1], [0.25, 0.25]);
        assert_eq!(d.leds[0][1], 0.75);
        assert!(d.leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }

    struct Paced;

    impl Driver for Paced {
        fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
            io.write(&[colors[0].0])
        }
        fn min_interval(&self) -> Duration {
            Duration::from_secs(3600)
        }
    }

    #[test]
    fn paced_frames_drop_but_black_always_goes_out() {
        let m = Mock::default();
        let dev = line("x".into(), "X", DeviceKind::Other, MOUSE, 1);
        let mut b = Hid::new("t", Box::new(m.clone()), Paced, dev);
        b.push("x", &[Color(5, 0, 0)]).unwrap();
        b.push("x", &[Color(6, 0, 0)]).unwrap();
        b.push("x", &[Color::BLACK]).unwrap();
        assert_eq!(m.take(), vec![Op::Write(vec![5]), Op::Write(vec![0])]);
    }
}
