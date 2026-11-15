//! Ported from OpenRGB Controllers (shared shell for the ASUS HID families) (GPL-2.0-or-later).

use crate::registry::{Detector, Info, Open};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

/// The HID calls a driver makes, so tests can record them instead of touching hardware.
pub trait Io: Send {
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    fn feature(&mut self, data: &[u8]) -> Result<(), String>;
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
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// One family's protocol: what to send to take control, and how to send a frame.
pub trait Proto: Send {
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Ok(())
    }
    /// `colors` has exactly one entry per LED of the device.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String>;
    /// Resend an unchanged frame after this long, for devices that drop out of direct mode.
    fn keepalive_ms(&self) -> Option<u64> {
        None
    }
}

pub struct Driver<P: Proto> {
    io: Box<dyn Io>,
    device: Device,
    name: &'static str,
    proto: P,
    started: bool,
    sent: Option<Vec<Color>>,
    at: std::time::Instant,
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
            at: std::time::Instant::now(),
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
        let stale = self
            .proto
            .keepalive_ms()
            .is_some_and(|ms| self.at.elapsed().as_millis() >= ms as u128);
        if self.sent.as_deref() == Some(&colors[..]) && !stale {
            return Ok(());
        }
        if !self.started {
            self.proto.start(self.io.as_mut())?;
            self.started = true;
        }
        match self.proto.frame(self.io.as_mut(), &colors) {
            Ok(()) => {
                self.sent = Some(colors);
                self.at = std::time::Instant::now();
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

/// Opens the collection and wraps it with `proto` and `device`.
pub fn boxed<P: Proto + 'static>(
    api: &HidApi,
    info: &Info,
    name: &'static str,
    device: Device,
    proto: P,
) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    Ok(Box::new(Driver::new(Box::new(hid), device, name, proto)))
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

/// `n` LEDs in a row.
pub fn line(info: &Info, family: &str, d: &Detector, kind: DeviceKind, n: usize) -> Device {
    Device::grid(
        &info.device_id(family),
        d.name,
        kind,
        rect(kind),
        n.max(1) as u32,
        1,
    )
}

/// LEDs in a `cols` x `rows` matrix at the given cells, in LED order.
pub fn cells(
    info: &Info,
    family: &str,
    d: &Detector,
    kind: DeviceKind,
    cols: u32,
    rows: u32,
    at: &[(u32, u32)],
) -> Device {
    let mut dev = Device::grid(
        &info.device_id(family),
        d.name,
        kind,
        rect(kind),
        cols,
        rows,
    );
    dev.leds = at
        .iter()
        .map(|&(c, r)| {
            [
                (c as f32 + 0.5) / cols as f32,
                (r as f32 + 0.5) / rows as f32,
            ]
        })
        .collect();
    dev
}

/// LEDs placed by an OpenRGB matrix map: `map[row][col]` is an LED index or `NA`.
pub fn mapped(
    info: &Info,
    family: &str,
    d: &Detector,
    kind: DeviceKind,
    map: &[&[u32]],
    n: usize,
) -> Device {
    let rows = map.len() as u32;
    let cols = map.iter().map(|r| r.len()).max().unwrap_or(1) as u32;
    let mut at = vec![(0, 0); n];
    for (r, row) in map.iter().enumerate() {
        for (c, &led) in row.iter().enumerate() {
            if (led as usize) < n {
                at[led as usize] = (c as u32, r as u32);
            }
        }
    }
    cells(info, family, d, kind, cols.max(1), rows.max(1), &at)
}

pub const NA: u32 = u32::MAX;

/// `n` LEDs on a ring, for fans.
pub fn ring(info: &Info, family: &str, d: &Detector, n: usize) -> Device {
    let mut dev = Device::grid(
        &info.device_id(family),
        d.name,
        DeviceKind::Other,
        rect(DeviceKind::Other),
        n.max(1) as u32,
        1,
    );
    dev.grid = [8, 8];
    dev.leds = (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [0.5 + 0.45 * a.sin(), 0.5 - 0.45 * a.cos()]
        })
        .collect();
    dev
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

/// One row per zone, in LED order.
pub fn stacked(
    info: &Info,
    family: &str,
    d: &Detector,
    kind: DeviceKind,
    zones: &[usize],
) -> Device {
    let cols = zones.iter().copied().max().unwrap_or(1).max(1) as u32;
    let at: Vec<(u32, u32)> = zones
        .iter()
        .enumerate()
        .flat_map(|(r, &n)| (0..n as u32).map(move |c| (c, r as u32)))
        .collect();
    cells(info, family, d, kind, cols, zones.len().max(1) as u32, &at)
}

/// Several backends behind one detector, for a collection OpenRGB builds two controllers on.
pub struct Both(pub Vec<Box<dyn Backend>>);

impl Backend for Both {
    fn name(&self) -> &str {
        self.0.first().map_or("", |b| b.name())
    }
    fn devices(&self) -> Vec<Device> {
        self.0.iter().flat_map(|b| b.devices()).collect()
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        match self
            .0
            .iter_mut()
            .find(|b| b.devices().iter().any(|d| d.id == device_id))
        {
            Some(b) => b.push(device_id, colors),
            None => Err(format!("unknown device {device_id}")),
        }
    }
}

/// ClearResponses: reads until nothing is waiting.
pub fn flush(io: &mut dyn Io) -> Result<(), String> {
    let mut buf = [0u8; 65];
    while io.read(&mut buf, 0)? > 0 {}
    Ok(())
}

/// A 65-byte report with `head` at the start.
pub fn report(head: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[..head.len()].copy_from_slice(head);
    b
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
        GetF(u8),
        Read,
        Sleep(u64),
    }

    #[derive(Default)]
    struct State {
        ops: Vec<Op>,
        reads: VecDeque<Vec<u8>>,
    }

    /// Records every call; reads and feature gets pop scripted replies.
    #[derive(Clone, Default)]
    pub struct Rec(Arc<Mutex<State>>);

    impl Rec {
        pub fn with(replies: Vec<Vec<u8>>) -> Rec {
            let r = Rec::default();
            r.0.lock().unwrap().reads = replies.into();
            r
        }
        pub fn take(&self) -> Vec<Op> {
            std::mem::take(&mut self.0.lock().unwrap().ops)
        }
        fn reply(&self, buf: &mut [u8]) -> usize {
            match self.0.lock().unwrap().reads.pop_front() {
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
            self.0.lock().unwrap().ops.push(Op::GetF(buf[0]));
            Ok(self.reply(buf))
        }
        fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            self.0.lock().unwrap().ops.push(Op::Read);
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
        assert!(again.is_empty(), "unchanged frame was sent again");
        [first, again, next]
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
