// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::{Duration, Instant};

use crate::registry::{Detector, Info, Open};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

/// The HID calls a driver makes, so tests can record them instead of touching hardware.
pub trait Io: Send {
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    fn feature(&mut self, data: &[u8]) -> Result<(), String>;
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
    fn feature(&mut self, data: &[u8]) -> Result<(), String> {
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

/// How long a blocking `hid_read` in OpenRGB may wait here.
pub const BLOCKING_MS: i32 = 1000;

/// Packets that keep the device listening: sent once the last frame is `idle` old, then every `every`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keepalive {
    pub idle: Duration,
    pub every: Duration,
}

/// One family's protocol: what to send to take control, a frame, keepalives and release.
pub trait Proto: Send {
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Ok(())
    }
    /// `colors` has exactly one entry per LED of the device.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String>;
    fn keepalive(&self) -> Option<Keepalive> {
        None
    }
    fn refresh(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Ok(())
    }
    /// Sent when the backend is dropped, as the controller's destructor does.
    fn stop(&mut self, _io: &mut dyn Io) {}
}

pub struct Driver<P: Proto> {
    io: Box<dyn Io>,
    device: Device,
    name: &'static str,
    proto: P,
    started: bool,
    sent: Option<Vec<Color>>,
    frame_at: Instant,
    kept_at: Option<Instant>,
}

impl<P: Proto> Driver<P> {
    pub fn new(io: Box<dyn Io>, device: Device, name: &'static str, proto: P) -> Self {
        Driver {
            io,
            device,
            name,
            proto,
            started: false,
            sent: None,
            frame_at: Instant::now(),
            kept_at: None,
        }
    }

    fn keep(&mut self) -> Result<(), String> {
        let Some(k) = self.proto.keepalive() else {
            return Ok(());
        };
        let due = self.frame_at.elapsed() >= k.idle
            && self.kept_at.is_none_or(|t| t.elapsed() >= k.every);
        if self.started && due {
            self.proto.refresh(self.io.as_mut())?;
            self.kept_at = Some(Instant::now());
        }
        Ok(())
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
        if self.sent.as_deref() != Some(&colors[..]) {
            let sent = (|| {
                if !self.started {
                    self.proto.start(self.io.as_mut())?;
                    self.started = true;
                    self.kept_at = Some(Instant::now());
                }
                self.proto.frame(self.io.as_mut(), &colors)
            })();
            if let Err(e) = sent {
                self.sent = None;
                return Err(e);
            }
            self.sent = Some(colors);
            self.frame_at = Instant::now();
        }
        self.keep()
    }
}

impl<P: Proto> Drop for Driver<P> {
    fn drop(&mut self) {
        self.proto.stop(self.io.as_mut());
    }
}

pub fn boxed<P: Proto + 'static>(
    io: Box<dyn Io>,
    name: &'static str,
    device: Device,
    proto: P,
) -> Box<dyn Backend> {
    Box::new(Driver::new(io, device, name, proto))
}

pub fn opened(api: &HidApi, info: &Info) -> Result<Box<dyn Io>, String> {
    Ok(Box::new(info.open(api)?))
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

/// A device with LEDs at the given 0..1 positions.
pub fn placed(
    info: &Info,
    family: &str,
    d: &Detector,
    kind: DeviceKind,
    grid: [u32; 2],
    leds: Vec<[f32; 2]>,
) -> Device {
    Device {
        id: info.device_id(family),
        name: d.name.into(),
        kind,
        rect: rect(kind),
        grid,
        leds,
    }
}

/// `n` LEDs in a row.
pub fn line(info: &Info, family: &str, d: &Detector, kind: DeviceKind, n: usize) -> Device {
    let leds = (0..n).map(|i| [(i as f32 + 0.5) / n as f32, 0.5]).collect();
    placed(info, family, d, kind, [n.max(1) as u32, 1], leds)
}

/// One row per zone, LEDs spread evenly along it.
pub fn rows(counts: &[usize]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let n = counts.len().max(1);
    let leds = counts
        .iter()
        .enumerate()
        .flat_map(|(r, &c)| {
            (0..c).map(move |i| [(i as f32 + 0.5) / c as f32, (r as f32 + 0.5) / n as f32])
        })
        .collect();
    let width = counts.iter().copied().max().unwrap_or(1).max(1);
    (leds, [width as u32, n as u32])
}

pub const NA: u32 = u32::MAX;

/// The LEDs an OpenRGB matrix map names, in LED order, with their cell centres.
pub fn mapped(map: &[&[u32]]) -> (Vec<u32>, Vec<[f32; 2]>, [u32; 2]) {
    let h = map.len();
    let w = map.iter().map(|r| r.len()).max().unwrap_or(1);
    let mut cells: Vec<(u32, [f32; 2])> = Vec::new();
    for (r, row) in map.iter().enumerate() {
        for (c, &led) in row.iter().enumerate() {
            if led != NA {
                let at = [(c as f32 + 0.5) / w as f32, (r as f32 + 0.5) / h as f32];
                cells.push((led, at));
            }
        }
    }
    cells.sort_by_key(|c| c.0);
    let (ids, at) = cells.into_iter().unzip();
    (ids, at, [w.max(1) as u32, h.max(1) as u32])
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

pub fn rgb(c: Color) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// `len` bytes, zero except for `head` at the start.
pub fn pad(head: &[u8], len: usize) -> Vec<u8> {
    let mut v = head.to_vec();
    v.resize(len, 0);
    v
}

#[cfg(test)]
pub mod rec {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Op {
        W(Vec<u8>),
        F(Vec<u8>),
        /// Report id and buffer length of a feature read.
        GetF(u8, usize),
        Read(usize),
        Sleep(u64),
    }

    #[derive(Default)]
    struct State {
        ops: Vec<Op>,
        replies: VecDeque<Vec<u8>>,
    }

    /// Records every call; reads and feature gets pop scripted replies.
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
        pub fn left(&self) -> usize {
            self.0.lock().unwrap().replies.len()
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
        fn write(&mut self, data: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().ops.push(Op::W(data.to_vec()));
            Ok(())
        }
        fn feature(&mut self, data: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().ops.push(Op::F(data.to_vec()));
            Ok(())
        }
        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            self.0.lock().unwrap().ops.push(Op::GetF(buf[0], buf.len()));
            Ok(self.reply(buf))
        }
        fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            self.0.lock().unwrap().ops.push(Op::Read(buf.len()));
            Ok(self.reply(buf))
        }
        fn sleep(&mut self, ms: u64) {
            self.0.lock().unwrap().ops.push(Op::Sleep(ms));
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

    pub fn w(bytes: Vec<u8>) -> Op {
        Op::W(bytes)
    }

    pub fn f(bytes: Vec<u8>) -> Op {
        Op::F(bytes)
    }

    #[test]
    fn keepalive_after_idle_then_every() {
        struct K;
        impl Proto for K {
            fn frame(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
                io.write(&[1])
            }
            fn keepalive(&self) -> Option<Keepalive> {
                Some(Keepalive {
                    idle: Duration::ZERO,
                    every: Duration::ZERO,
                })
            }
            fn refresh(&mut self, io: &mut dyn Io) -> Result<(), String> {
                io.write(&[2])
            }
        }
        let [first, again, _] = pushes(K, 1, vec![], &[Color::BLACK], &[Color::BLACK]);
        assert_eq!(first, vec![w(vec![1]), w(vec![2])]);
        assert_eq!(again, vec![w(vec![2])]);
    }

    #[test]
    fn mapped_orders_by_led() {
        let (ids, at, grid) = mapped(&[&[3, NA], &[0, 1]]);
        assert_eq!(ids, vec![0, 1, 3]);
        assert_eq!(grid, [2, 2]);
        assert_eq!(at, vec![[0.25, 0.75], [0.75, 0.75], [0.25, 0.25]]);
    }
}
