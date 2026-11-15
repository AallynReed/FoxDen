//! Shared WinUSB handles for Lian Li raw-USB and hybrid devices (protocol learned from SignalRGB's Lian Li plugins).

use foxrgb_core::{Backend, Color, Device};
use foxrgb_hid::winusb::{self, WinUsb};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Usb>>;

const VENDOR_OUT: u8 = 0x40;
const VENDOR_IN: u8 = 0xC0;
const REQ_WRITE: u8 = 0x80;
const REQ_READ: u8 = 0x81;

/// One device path bound to WinUSB.
#[derive(Clone, Debug)]
pub struct UsbDevice {
    vid: u16,
    pid: u16,
    path: String,
}

/// One opened device, shared by its lighting backend and its fan hub.
pub struct Usb {
    handle: Mutex<WinUsb>,
    pub vid: u16,
    pub pid: u16,
    pub serial: String,
    pub device: Arc<foxrgb_hid::port::Device>,
    key: String,
}

impl Usb {
    fn handle(&self) -> Result<std::sync::MutexGuard<'_, WinUsb>, String> {
        self.handle.lock().map_err(|e| e.to_string())
    }

    /// Vendor write (0x40, request 0x80, value 0, index = register address).
    pub fn ctrl_out(&self, index: u16, data: &[u8]) -> Result<(), String> {
        let mut b = data.to_vec();
        self.handle()?
            .control_data(VENDOR_OUT, REQ_WRITE, 0, index, &mut b)
            .map(|_| ())
    }

    /// Vendor read (0xC0, request 0x81, value 0, index = register address).
    pub fn ctrl_in(&self, index: u16, len: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        let n = self
            .handle()?
            .control_data(VENDOR_IN, REQ_READ, 0, index, &mut b)?;
        b.truncate(n);
        Ok(b)
    }

    /// A HID output report over the control pipe, for a HID interface WinUSB took over.
    pub fn set_report(&self, interface: u8, report: &[u8]) -> Result<(), String> {
        self.handle()?.set_report(interface, report)
    }

    pub fn product(&self) -> Option<String> {
        self.handle().ok()?.product()
    }

    /// Bulk write on the endpoint the handle was opened with.
    pub fn bulk_out(&self, _ep: u8, data: &[u8], _timeout_ms: u64) -> Result<(), String> {
        self.handle()?.write_pipe(data)
    }

    /// One bulk read; empty on timeout.
    pub fn bulk_in(&self, _ep: u8, len: usize, timeout_ms: u64) -> Result<Vec<u8>, String> {
        self.handle()?.read_pipe(len, timeout_ms as u32)
    }

    /// Serial when the device has one, else its WinUSB instance: stable across runs on one machine.
    pub fn tail(&self) -> String {
        if self.serial.is_empty() {
            self.key.clone()
        } else {
            self.serial.clone()
        }
    }

    pub fn device_id(&self, family: &str) -> String {
        format!(
            "usb:{family}-{:04x}{:04x}-{}",
            self.vid,
            self.pid,
            self.tail()
        )
    }
}

impl UsbDevice {
    /// The serial in the path, empty when Windows made the instance up.
    pub fn serial(&self) -> String {
        let instance = winusb::instance(&self.path);
        if instance.contains('&') {
            String::new()
        } else {
            instance.to_string()
        }
    }

    fn key(&self) -> String {
        winusb::instance(&self.path).replace('&', ".")
    }

    /// Matches `Usb::device_id` with the `fan:` prefix, open or not.
    pub fn fan_id(&self, family: &str) -> String {
        let serial = self.serial();
        let tail = if serial.is_empty() {
            self.key()
        } else {
            serial
        };
        format!("fan:{family}-{:04x}{:04x}-{tail}", self.vid, self.pid)
    }
}

fn registry() -> &'static Mutex<HashMap<String, Weak<Mutex<Usb>>>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Weak<Mutex<Usb>>>>> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

/// The device lock of a path, the same before and after it opens.
pub fn lock_for(dev: &UsbDevice) -> Arc<foxrgb_hid::port::Device> {
    static LOCKS: Mutex<Vec<(String, Weak<foxrgb_hid::port::Device>)>> = Mutex::new(Vec::new());
    let full = dev.path.to_ascii_lowercase();
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.retain(|(_, d)| d.strong_count() > 0);
    if let Some(d) = locks
        .iter()
        .find_map(|(p, d)| (*p == full).then(|| d.upgrade()).flatten())
    {
        return d;
    }
    let d = Arc::default();
    locks.push((full, Arc::downgrade(&d)));
    d
}

/// Every attached device with these ids that is bound to WinUSB.
pub fn find(vid: u16, pid: u16) -> Vec<UsbDevice> {
    WinUsb::paths(vid, pid)
        .into_iter()
        .map(|path| UsbDevice { vid, pid, path })
        .collect()
}

/// The devices WinUSB is bound to, including ones another process holds open.
pub fn find_bound(vid: u16, pid: u16) -> Vec<UsbDevice> {
    WinUsb::bound_paths(vid, pid)
        .into_iter()
        .map(|path| UsbDevice { vid, pid, path })
        .collect()
}

/// Opens a device, or hands back the handle another backend already holds. With `bulk_ep` the
/// bound interface must own that endpoint, its IN partner being `bulk_ep | 0x80`.
pub fn open(dev: &UsbDevice, bulk_ep: Option<u8>) -> Result<Shared, String> {
    let full = dev.path.to_ascii_lowercase();
    let mut reg = registry().lock().map_err(|e| e.to_string())?;
    if let Some(s) = reg.get(&full).and_then(Weak::upgrade) {
        return Ok(s);
    }
    let out = bulk_ep.unwrap_or(0x01);
    let handle = WinUsb::at(&dev.path, out, out | 0x80)?;
    if bulk_ep.is_some_and(|ep| !handle.has_pipe(ep)) {
        return Err("bulk endpoint not found".into());
    }
    let s = Arc::new(Mutex::new(Usb {
        handle: Mutex::new(handle),
        vid: dev.vid,
        pid: dev.pid,
        serial: dev.serial(),
        key: dev.key(),
        device: lock_for(dev),
    }));
    reg.insert(full, Arc::downgrade(&s));
    Ok(s)
}

/// The USB side of a hybrid hub whose HID collection has this serial. Without serials it pairs
/// only when exactly one such device is attached, so one hub never reports another's rpm.
pub fn pair(vid: u16, pid: u16, serial: &str) -> Option<Shared> {
    let devs = find(vid, pid);
    if serial.is_empty() {
        return match devs.as_slice() {
            [one] => open(one, None).ok(),
            _ => None,
        };
    }
    for d in &devs {
        if let Ok(s) = open(d, None)
            && s.lock()
                .is_ok_and(|u| u.serial.eq_ignore_ascii_case(serial))
        {
            return Some(s);
        }
    }
    None
}

/// One raw-USB lighting protocol.
pub trait UsbProto: Send {
    fn start(&mut self, _usb: &Usb) -> Result<(), String> {
        Ok(())
    }
    /// `colors` has one entry per LED, in `Device::leds` order.
    fn frame(&mut self, usb: &Usb, colors: &[Color]) -> Result<(), String>;
    fn keepalive_ms(&self) -> Option<u64> {
        None
    }
    fn stop(&mut self, _usb: &Usb) {}
}

/// A lighting backend over a shared raw-USB handle.
pub struct UsbDriver<P: UsbProto> {
    usb: Shared,
    device: Device,
    family: &'static str,
    proto: P,
    started: bool,
    sent: Option<Vec<Color>>,
    at: Instant,
}

impl<P: UsbProto + 'static> UsbDriver<P> {
    pub fn boxed(usb: Shared, device: Device, family: &'static str, proto: P) -> Box<dyn Backend> {
        Box::new(UsbDriver {
            usb,
            device,
            family,
            proto,
            started: false,
            sent: None,
            at: Instant::now(),
        })
    }
}

impl<P: UsbProto> Backend for UsbDriver<P> {
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
        let usb = self.usb.lock().map_err(|e| e.to_string())?;
        if !self.started {
            self.proto.start(&usb)?;
            self.started = true;
        }
        let stale = self
            .proto
            .keepalive_ms()
            .is_some_and(|v| self.at.elapsed() >= Duration::from_millis(v));
        if self.sent.as_deref() == Some(colors) && !stale {
            return Ok(());
        }
        let mut c = colors.to_vec();
        c.resize(self.device.leds.len(), Color(0, 0, 0));
        self.proto.frame(&usb, &c)?;
        self.sent = Some(colors.to_vec());
        self.at = Instant::now();
        Ok(())
    }
}

impl<P: UsbProto> Drop for UsbDriver<P> {
    fn drop(&mut self) {
        if self.started
            && let Ok(usb) = self.usb.lock()
        {
            self.proto.stop(&usb);
        }
    }
}

pub fn pause(v: u64) {
    std::thread::sleep(Duration::from_millis(v));
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn at(pid: u16, instance: &str) -> UsbDevice {
        UsbDevice {
            vid: 0x0CF2,
            pid,
            path: format!(
                r"\\?\usb#vid_0cf2&pid_{pid:04x}#{instance}#{{a5dcbf10-6530-11d2-901f-00c04fb951ed}}"
            ),
        }
    }

    // A device another process holds keeps the id and lock it gets once it opens.
    #[test]
    fn a_device_has_its_fan_id_and_lock_before_it_opens() {
        let held = at(0xA100, "ABC123");
        assert_eq!(held.fan_id("f"), "fan:f-0cf2a100-ABC123");
        let bare = at(0xA100, "6&1a2b&0&2");
        assert_eq!(bare.serial(), "");
        assert_eq!(bare.fan_id("f"), "fan:f-0cf2a100-6.1a2b.0.2");
        assert!(Arc::ptr_eq(&lock_for(&held), &lock_for(&held.clone())));
        assert!(!Arc::ptr_eq(&lock_for(&held), &lock_for(&bare)));
        assert!(open(&held, None).is_err());
    }
}
