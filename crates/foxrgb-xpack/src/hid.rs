//! Shared HID plumbing for every lighting driver: the calls a driver makes and a backend shell.

use foxrgb_core::{Backend, Color, Device};
use foxrgb_hid::port::{LIGHTS, Port};
use foxrgb_hid::registry::{Detector, Info, Open};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

/// The HID calls a driver makes. `data[0]` is always the report id (0 when the device has none).
pub trait Io: Send {
    /// Output report, zero padded to `len` bytes.
    fn write(&mut self, data: &[u8], len: usize) -> Result<(), String>;
    /// Feature report, zero padded to `len` bytes.
    fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String>;
    /// Feature report read: `first` is the report id, the reply is `len` bytes long.
    fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String>;
    /// One input report, or an empty vector when nothing came within `timeout_ms`.
    fn read(&mut self, len: usize, timeout_ms: i32) -> Result<Vec<u8>, String>;
    fn sleep(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    /// The shared handle behind this `Io`, when there is one.
    fn port(&self) -> Option<&Port> {
        None
    }
}

pub fn padded(data: &[u8], len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len.max(data.len())];
    b[..data.len()].copy_from_slice(data);
    b.truncate(len.max(1));
    b
}

impl Io for HidDevice {
    fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        HidDevice::write(self, &padded(data, len))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        self.send_feature_report(&padded(data, len))
            .map_err(|e| e.to_string())
    }
    fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        b[0] = first;
        let n = self.get_feature_report(&mut b).map_err(|e| e.to_string())?;
        b.truncate(n);
        Ok(b)
    }
    fn read(&mut self, len: usize, timeout_ms: i32) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        let n = self
            .read_timeout(&mut b, timeout_ms)
            .map_err(|e| e.to_string())?;
        b.truncate(n);
        Ok(b)
    }
}

impl Io for Port {
    fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        Port::write(self, &padded(data, len))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        self.send_feature_report(&padded(data, len))
            .map_err(|e| e.to_string())
    }
    fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        b[0] = first;
        let n = self.get_feature_report(&mut b).map_err(|e| e.to_string())?;
        b.truncate(n);
        Ok(b)
    }
    fn read(&mut self, len: usize, timeout_ms: i32) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        let n = self
            .read_timeout(&mut b, timeout_ms)
            .map_err(|e| e.to_string())?;
        b.truncate(n);
        Ok(b)
    }
    fn port(&self) -> Option<&Port> {
        Some(self)
    }
}

/// Reads input reports until one passes `want` or `timeout_ms` runs out.
pub fn read_until(
    io: &mut dyn Io,
    len: usize,
    timeout_ms: u64,
    want: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    let end = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < end {
        let left = end.saturating_duration_since(Instant::now()).as_millis() as i32;
        let r = io.read(len, left.max(1))?;
        if !r.is_empty() && want(&r) {
            return Ok(r);
        }
    }
    Err("no reply".into())
}

/// Drops every input report already waiting.
pub fn drain(io: &mut dyn Io, len: usize) {
    while matches!(io.read(len, 0), Ok(r) if !r.is_empty()) {}
}

/// Opens a sibling collection of the same physical device, picked by interface or usage page.
pub fn open_sibling(
    api: &HidApi,
    info: &Info,
    interface: Option<i32>,
    usage_page: Option<u16>,
    usage: Option<u16>,
) -> Result<HidDevice, String> {
    for raw in foxrgb_hid::device_list(api).iter() {
        let i = Info::from_hidapi(raw);
        if i.vid == info.vid
            && i.pid == info.pid
            && i.serial == info.serial
            && interface.is_none_or(|x| x == i.interface)
            && usage_page.is_none_or(|x| x == i.usage_page)
            && usage.is_none_or(|x| x == i.usage)
        {
            return i.open(api);
        }
    }
    Err("collection not found".into())
}

/// One family's protocol: what to send to take control, and how to send a frame.
pub trait Proto: Send {
    fn start(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Ok(())
    }
    /// `colors` has exactly one entry per LED of the device, in `Device::leds` order.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String>;
    /// Resend an unchanged frame after this long, for devices that fall back to their own effect.
    fn keepalive_ms(&self) -> Option<u64> {
        None
    }
    /// Hands the device back to its firmware effect when FoxRGB lets go.
    fn stop(&mut self, _io: &mut dyn Io) {}
}

/// A lighting backend for one device: starts it once, sends changed frames and keepalives.
pub struct Driver<P: Proto> {
    io: Box<dyn Io>,
    device: Device,
    family: &'static str,
    proto: P,
    started: bool,
    sent: Option<Vec<Color>>,
    at: Instant,
}

impl<P: Proto> Driver<P> {
    pub fn new(io: Box<dyn Io>, device: Device, family: &'static str, proto: P) -> Self {
        Driver {
            io,
            device,
            family,
            proto,
            started: false,
            sent: None,
            at: Instant::now(),
        }
    }

    pub fn boxed(self) -> Box<dyn Backend>
    where
        P: 'static,
    {
        Box::new(self)
    }
}

impl<P: Proto> Backend for Driver<P> {
    fn name(&self) -> &str {
        self.family
    }
    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Ok(());
        }
        if !self.started {
            self.proto.start(self.io.as_mut())?;
            self.started = true;
            self.tell(true);
        }
        let stale = self
            .proto
            .keepalive_ms()
            .is_some_and(|ms| self.at.elapsed() >= Duration::from_millis(ms));
        if self.sent.as_deref() == Some(colors) && !stale {
            return Ok(());
        }
        let mut c = colors.to_vec();
        c.resize(self.device.leds.len(), Color(0, 0, 0));
        self.proto.frame(self.io.as_mut(), &c)?;
        self.sent = Some(colors.to_vec());
        self.at = Instant::now();
        Ok(())
    }
}

impl<P: Proto> Driver<P> {
    /// Tells a shared device's fan driver that lighting took it or let go.
    fn tell(&self, on: bool) {
        if let Some(p) = self.io.port() {
            p.device().set_active(LIGHTS, on);
            p.device().bump();
        }
    }
}

impl<P: Proto> Drop for Driver<P> {
    fn drop(&mut self) {
        if self.started {
            self.proto.stop(self.io.as_mut());
            self.tell(false);
        }
    }
}

pub fn rgb(c: Color) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// A detector on any collection of the device.
pub const fn any(name: &'static str, vid: u16, pid: u16, open: Open) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: None,
        usage_page: None,
        usage: None,
        open,
    }
}

/// A detector on one interface.
pub const fn on_i(name: &'static str, vid: u16, pid: u16, interface: i32, open: Open) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: Some(interface),
        usage_page: None,
        usage: None,
        open,
    }
}

/// A detector on one usage page (and usage when given).
pub const fn on_p(
    name: &'static str,
    vid: u16,
    pid: u16,
    page: u16,
    usage: Option<u16>,
    open: Open,
) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: None,
        usage_page: Some(page),
        usage,
        open,
    }
}

/// Opens the collection a detector matched as a boxed `Io`.
pub fn open_io(api: &HidApi, info: &Info) -> Result<Box<dyn Io>, String> {
    Ok(Box::new(info.port(api)?))
}

#[cfg(test)]
pub mod wire {
    use foxrgb_hid::port::{HidResult, Port, Raw};
    use std::sync::{Arc, Mutex};

    /// One handle that records every write; reads find nothing.
    #[derive(Clone, Default)]
    pub struct Wire(pub Arc<Mutex<Vec<Vec<u8>>>>);

    impl Wire {
        pub fn port(&self) -> Port {
            Port::alone(Box::new(self.clone()))
        }
        pub fn take(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    impl Raw for Wire {
        fn write(&self, data: &[u8]) -> HidResult<usize> {
            self.0.lock().unwrap().push(data.to_vec());
            Ok(data.len())
        }
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            Ok(0)
        }
        fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
            self.0.lock().unwrap().push(data.to_vec());
            Ok(())
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
        fn get_input_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
    }
}
