//! Shared helpers for the lights_c drivers: device layouts from SignalRGB sizes and positions.

use crate::hid::{Driver, Proto, open_io};
use foxden_core::{Backend, Device, DeviceKind};
use foxden_hid::registry::Info;
use hidapi::HidApi;

/// Default canvas placement for each kind of device.
pub fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        DeviceKind::Strip => [0.06, 0.12, 0.52, 0.18],
        _ => [0.62, 0.30, 0.30, 0.12],
    }
}

/// LEDs at SignalRGB `LedPositions` cells of a `Size()` of `w` x `h`, in plugin order.
pub fn positioned(
    id: &str,
    name: &str,
    kind: DeviceKind,
    w: u32,
    h: u32,
    pos: &[(u8, u8)],
) -> Device {
    let (w, h) = (w.max(1), h.max(1));
    let mut d = Device::grid(id, name, kind, rect(kind), w, h);
    d.leds = pos
        .iter()
        .map(|&(x, y)| [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32])
        .collect();
    d
}

/// `n` LEDs in one row.
pub fn line(id: &str, name: &str, kind: DeviceKind, n: usize) -> Device {
    Device::grid(id, name, kind, rect(kind), n.max(1) as u32, 1)
}

/// Opens the matched collection and wraps `proto` in a lighting backend.
pub fn open_with<P: Proto + 'static>(
    api: &HidApi,
    info: &Info,
    family: &'static str,
    dev: Device,
    proto: P,
) -> Result<Box<dyn Backend>, String> {
    Ok(Driver::new(open_io(api, info)?, dev, family, proto).boxed())
}

#[cfg(test)]
pub mod mock {
    use crate::hid::{Io, padded};
    use std::collections::VecDeque;

    #[derive(Clone, Debug, PartialEq)]
    pub enum Op {
        Write(Vec<u8>),
        Feature(Vec<u8>),
        GetFeature(u8, usize),
        Read(usize),
    }

    /// Records every call; `replies` answers get_feature and read in order.
    #[derive(Default)]
    pub struct Rec {
        pub ops: Vec<Op>,
        pub replies: VecDeque<Vec<u8>>,
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.ops.push(Op::Write(padded(data, len)));
            Ok(())
        }
        fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.ops.push(Op::Feature(padded(data, len)));
            Ok(())
        }
        fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String> {
            self.ops.push(Op::GetFeature(first, len));
            Ok(self.replies.pop_front().unwrap_or_else(|| {
                let mut b = vec![0u8; len];
                b[0] = first;
                b
            }))
        }
        fn read(&mut self, len: usize, _timeout_ms: i32) -> Result<Vec<u8>, String> {
            self.ops.push(Op::Read(len));
            Ok(self.replies.pop_front().unwrap_or_default())
        }
        fn sleep(&mut self, _ms: u64) {}
    }
}
