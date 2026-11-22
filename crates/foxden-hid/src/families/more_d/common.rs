// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::{Duration, Instant};

use crate::registry::{Detector, Info, Open};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;

const UNSUPPORTED: &str = "not supported on this transport";

/// Every call a driver makes on HID, serial, USB or SCSI, so tests can record them.
pub trait Io: Send {
    /// HID output report, serial write, bulk OUT or SCSI write; returns the bytes taken.
    fn write(&mut self, data: &[u8]) -> Result<usize, String>;
    /// HID input report, serial read or bulk IN.
    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String>;
    fn feature(&mut self, _data: &[u8]) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    /// `buf[0]` holds the report id; returns the length read.
    fn get_feature(&mut self, _buf: &mut [u8]) -> Result<usize, String> {
        Err(UNSUPPORTED.into())
    }
    /// A control transfer with no data stage.
    fn control(&mut self, _kind: u8, _request: u8, _value: u16, _index: u16) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    fn reset(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Drops pending serial input and output.
    fn flush(&mut self, _rx: bool, _tx: bool) {}
    fn sleep(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

impl Io for HidDevice {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        HidDevice::write(self, data).map_err(|e| e.to_string())
    }
    fn read(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, String> {
        self.read_timeout(buf, timeout_ms)
            .map_err(|e| e.to_string())
    }
    fn feature(&mut self, data: &[u8]) -> Result<(), String> {
        self.send_feature_report(data).map_err(|e| e.to_string())
    }
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.get_feature_report(buf).map_err(|e| e.to_string())
    }
}

/// One family's protocol: what to send to take control, a frame and its keepalive.
pub trait Proto: Send {
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Ok(())
    }
    /// `colors` has exactly one entry per LED of the device.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String>;
    /// How long an unchanged frame may go unsent.
    fn keepalive_ms(&self) -> Option<u64> {
        None
    }
    fn refresh(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.frame(io, colors)
    }
}

pub struct Driver<P: Proto> {
    io: Box<dyn Io>,
    device: Device,
    name: &'static str,
    proto: P,
    sent: Option<Vec<Color>>,
    started: bool,
    at: Instant,
}

impl<P: Proto> Driver<P> {
    pub fn new(io: Box<dyn Io>, device: Device, name: &'static str, proto: P) -> Self {
        Driver {
            io,
            device,
            name,
            proto,
            sent: None,
            started: false,
            at: Instant::now(),
        }
    }
}

impl<P: Proto> Backend for Driver<P> {
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
        let io = self.io.as_mut();
        let result = if self.sent.as_deref() == Some(&colors[..]) {
            let stale = self
                .proto
                .keepalive_ms()
                .is_some_and(|ms| self.at.elapsed() >= Duration::from_millis(ms));
            if !stale {
                return Ok(());
            }
            self.proto.refresh(io, &colors)
        } else if !self.started {
            self.proto
                .start(io)
                .and_then(|()| self.proto.frame(io, &colors))
        } else {
            self.proto.frame(io, &colors)
        };
        match result {
            Ok(()) => {
                self.started = true;
                self.sent = Some(colors);
                self.at = Instant::now();
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

pub fn boxed<P: Proto + 'static>(
    io: Box<dyn Io>,
    device: Device,
    name: &'static str,
    proto: P,
) -> Box<dyn Backend> {
    Box::new(Driver::new(io, device, name, proto))
}

pub fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        DeviceKind::Strip => [0.06, 0.45, 0.60, 0.04],
        DeviceKind::Motherboard => [0.06, 0.10, 0.52, 0.15],
        DeviceKind::Gpu => [0.06, 0.30, 0.40, 0.05],
        DeviceKind::Memory => [0.68, 0.10, 0.03, 0.36],
        DeviceKind::Other => [0.82, 0.30, 0.12, 0.12],
    }
}

/// Each zone a row, LEDs spread evenly along it.
pub fn rows(id: String, name: &str, kind: DeviceKind, counts: &[usize]) -> Device {
    let n = counts.len().max(1);
    let width = counts.iter().copied().max().unwrap_or(1).max(1);
    let leds = counts
        .iter()
        .enumerate()
        .flat_map(|(r, &c)| {
            (0..c).map(move |i| [(i as f32 + 0.5) / c as f32, (r as f32 + 0.5) / n as f32])
        })
        .collect();
    Device {
        id,
        name: name.into(),
        kind,
        rect: rect(kind),
        grid: [width as u32, n as u32],
        leds,
    }
}

pub const NA: u32 = u32::MAX;

/// LEDs placed by an OpenRGB matrix map (`map[row * width + col]`); unmapped ones go below it.
pub fn mapped(
    id: String,
    name: &str,
    kind: DeviceKind,
    map: &[u32],
    width: usize,
    n: usize,
) -> Device {
    let height = (map.len() / width.max(1)).max(1);
    let rows_total = height + 1;
    let leds = (0..n)
        .map(|led| match map.iter().position(|&m| m == led as u32) {
            Some(cell) => [
                ((cell % width) as f32 + 0.5) / width as f32,
                ((cell / width) as f32 + 0.5) / rows_total as f32,
            ],
            None => [
                (led as f32 + 0.5) / n as f32,
                (height as f32 + 0.5) / rows_total as f32,
            ],
        })
        .collect();
    Device {
        id,
        name: name.into(),
        kind,
        rect: rect(kind),
        grid: [width as u32, rows_total as u32],
        leds,
    }
}

pub const ANY: i32 = -1;

/// A detector as a `REGISTER_HID_DETECTOR*` line gives it; `ANY` leaves a field open.
pub const fn det(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: i32,
    page: i32,
    usage: i32,
    open: Open,
) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: if interface < 0 { None } else { Some(interface) },
        usage_page: if page < 0 { None } else { Some(page as u16) },
        usage: if usage < 0 { None } else { Some(usage as u16) },
        open,
    }
}

pub fn hid_id(info: &Info, family: &str) -> String {
    info.device_id(family)
}

/// A device id for something reached by a name such as `COM3`.
pub fn named_id(family: &str, name: &str) -> String {
    let tail: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    format!("hid:{family}-{tail}")
}

#[cfg(test)]
pub mod rec {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Op {
        W(Vec<u8>),
        R(usize),
        F(Vec<u8>),
        /// The buffer handed to `hid_get_feature_report`, as it was on entry.
        GetF(Vec<u8>),
        Ctl(u8, u8, u16, u16),
        Reset,
        Flush(bool, bool),
        Sleep(u64),
    }

    #[derive(Default)]
    struct State {
        ops: Vec<Op>,
        replies: VecDeque<Vec<u8>>,
    }

    /// Records every call; reads and feature gets pop scripted replies, or read nothing.
    #[derive(Clone, Default)]
    pub struct Rec(Arc<Mutex<State>>);

    impl Rec {
        pub fn with(replies: Vec<Vec<u8>>) -> Rec {
            let r = Rec::default();
            r.0.lock().unwrap().replies = replies.into();
            r
        }
        pub fn take(&self) -> Vec<Op> {
            std::mem::take(&mut self.0.lock().unwrap().ops)
        }
        fn log(&self, op: Op) {
            self.0.lock().unwrap().ops.push(op);
        }
        fn reply(&self, buf: &mut [u8]) -> usize {
            match self.0.lock().unwrap().replies.pop_front() {
                Some(r) => {
                    let n = r.len().min(buf.len());
                    buf[..n].copy_from_slice(&r[..n]);
                    n
                }
                None => 0,
            }
        }
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8]) -> Result<usize, String> {
            self.log(Op::W(data.to_vec()));
            Ok(data.len())
        }
        fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            self.log(Op::R(buf.len()));
            Ok(self.reply(buf))
        }
        fn feature(&mut self, data: &[u8]) -> Result<(), String> {
            self.log(Op::F(data.to_vec()));
            Ok(())
        }
        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            self.log(Op::GetF(buf.to_vec()));
            Ok(self.reply(buf))
        }
        fn control(&mut self, kind: u8, request: u8, value: u16, index: u16) -> Result<(), String> {
            self.log(Op::Ctl(kind, request, value, index));
            Ok(())
        }
        fn reset(&mut self) -> Result<(), String> {
            self.log(Op::Reset);
            Ok(())
        }
        fn flush(&mut self, rx: bool, tx: bool) {
            self.log(Op::Flush(rx, tx));
        }
        fn sleep(&mut self, ms: u64) {
            self.log(Op::Sleep(ms));
        }
    }

    pub fn dev(n: usize) -> Device {
        Device::grid(
            "t",
            "t",
            DeviceKind::Other,
            [0.0, 0.0, 1.0, 1.0],
            n as u32,
            1,
        )
    }

    /// Pushes `a`, `a` again, then `b`; returns what each push sent.
    pub fn pushes<P: Proto>(
        proto: P,
        n: usize,
        replies: Vec<Vec<u8>>,
        a: &[Color],
        b: &[Color],
    ) -> [Vec<Op>; 3] {
        let rec = Rec::with(replies);
        let mut d = Driver::new(Box::new(rec.clone()), dev(n), "t", proto);
        d.push("t", a).unwrap();
        let first = rec.take();
        d.push("t", a).unwrap();
        let again = rec.take();
        d.push("t", b).unwrap();
        let next = rec.take();
        [first, again, next]
    }

    /// Runs `f` against a recorder and returns what it sent.
    pub fn record(replies: Vec<Vec<u8>>, f: impl FnOnce(&mut dyn Io)) -> Vec<Op> {
        let rec = Rec::with(replies);
        let mut io = rec.clone();
        f(&mut io);
        rec.take()
    }

    pub fn w(bytes: Vec<u8>) -> Op {
        Op::W(bytes)
    }

    /// `len` bytes, zero except for `head` at the start.
    pub fn pad(head: &[u8], len: usize) -> Vec<u8> {
        let mut v = head.to_vec();
        v.resize(len, 0);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::rec::*;
    use super::*;

    struct Count;
    impl Proto for Count {
        fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
            io.write(&[0]).map(|_| ())
        }
        fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
            io.write(&[1, colors[0].0]).map(|_| ())
        }
        fn keepalive_ms(&self) -> Option<u64> {
            Some(0)
        }
        fn refresh(&mut self, io: &mut dyn Io, _colors: &[Color]) -> Result<(), String> {
            io.write(&[2]).map(|_| ())
        }
    }

    #[test]
    fn start_once_then_frames_and_refreshes() {
        let [first, again, next] = pushes(Count, 1, vec![], &[Color(5, 0, 0)], &[Color(6, 0, 0)]);
        assert_eq!(first, vec![w(vec![0]), w(vec![1, 5])]);
        assert_eq!(again, vec![w(vec![2])]);
        assert_eq!(next, vec![w(vec![1, 6])]);
    }

    #[test]
    fn named_ids_are_stable() {
        assert_eq!(named_id("arctic", "COM12"), "hid:arctic-com12");
    }
}
