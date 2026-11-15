//! Helpers shared by the fans_a fan hubs and lighting drivers.

use crate::hid::Io;
use foxrgb_core::{Device, DeviceKind};
use foxrgb_hid::registry::{Bus, Info};
use hidapi::HidApi;
use std::time::{Duration, Instant};

/// Stable fan hub id: the HID device id with a `fan:` prefix.
pub fn hub_id(info: &Info, family: &str) -> String {
    info.device_id(family).replacen("hid:", "fan:", 1)
}

/// Every USB collection passing `want`, one per physical device not yet in `seen`; vendor pages win.
pub fn scan(api: &HidApi, seen: &mut Vec<String>, want: impl Fn(&Info) -> bool) -> Vec<Info> {
    let mut all: Vec<Info> = api
        .device_list()
        .map(Info::from_hidapi)
        .filter(|i| i.bus == Bus::Usb && want(i))
        .collect();
    all.sort_by_key(|i| i.usage_page < 0xFF00);
    let mut out = Vec::new();
    for info in all {
        if !seen.contains(&info.key()) {
            seen.push(info.key());
            out.push(info);
        }
    }
    out
}

/// Interface `n`; a device with a single interface lists it as -1.
pub fn iface(info: &Info, n: i32) -> bool {
    info.interface == n || (n == 0 && info.interface == -1)
}

pub fn open(api: &HidApi, info: &Info) -> Result<Box<dyn Io>, String> {
    Ok(Box::new(info.port(api)?))
}

/// True at most once per period; the first call is always due.
pub struct Every {
    at: Option<Instant>,
    period: Duration,
}

impl Every {
    pub fn ms(ms: u64) -> Self {
        Every {
            at: None,
            period: Duration::from_millis(ms),
        }
    }
    pub fn due(&mut self) -> bool {
        if self.at.is_some_and(|t| t.elapsed() < self.period) {
            return false;
        }
        self.at = Some(Instant::now());
        true
    }
}

/// The newest queued input report passing `want`, without waiting.
pub fn latest(io: &mut dyn Io, len: usize, want: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let mut last = None;
    while let Ok(r) = io.read(len, 0) {
        if r.is_empty() {
            break;
        }
        if want(&r) {
            last = Some(r);
        }
    }
    last
}

/// Rpm of a channel that has spun at least once; `None` while it never has.
pub fn spun(seen: &mut bool, rpm: u32) -> Option<u32> {
    *seen |= rpm > 0;
    seen.then_some(rpm)
}

/// A device with LEDs at whole cells of a `cols` x `rows` grid.
pub fn cells(
    id: &str,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    size: [u32; 2],
    at: &[[u8; 2]],
) -> Device {
    let [w, h] = [size[0].max(1), size[1].max(1)];
    let mut dev = Device::grid(id, name, kind, rect, w, h);
    dev.leds = at
        .iter()
        .map(|[x, y]| [(*x as f32 + 0.5) / w as f32, (*y as f32 + 0.5) / h as f32])
        .collect();
    dev
}

pub fn le16(r: &[u8], at: usize) -> u32 {
    r.get(at).copied().unwrap_or(0) as u32 | (r.get(at + 1).copied().unwrap_or(0) as u32) << 8
}

pub fn be16(r: &[u8], at: usize) -> u32 {
    (r.get(at).copied().unwrap_or(0) as u32) << 8 | r.get(at + 1).copied().unwrap_or(0) as u32
}

#[cfg(test)]
pub mod mock {
    use crate::hid::Io;
    use std::sync::{Arc, Mutex};

    /// Records writes and feature reports; replies come from `replies` in order.
    #[derive(Clone, Default)]
    pub struct Rec {
        pub sent: Arc<Mutex<Vec<Vec<u8>>>>,
        pub replies: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push(crate::hid::padded(data, len));
            Ok(())
        }
        fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push(crate::hid::padded(data, len));
            Ok(())
        }
        fn get_feature(&mut self, _first: u8, _len: usize) -> Result<Vec<u8>, String> {
            let mut r = self.replies.lock().unwrap();
            Ok(if r.is_empty() {
                Vec::new()
            } else {
                r.remove(0)
            })
        }
        fn read(&mut self, _len: usize, _timeout_ms: i32) -> Result<Vec<u8>, String> {
            let mut r = self.replies.lock().unwrap();
            Ok(if r.is_empty() {
                Vec::new()
            } else {
                r.remove(0)
            })
        }
        fn sleep(&mut self, _ms: u64) {}
    }
}
