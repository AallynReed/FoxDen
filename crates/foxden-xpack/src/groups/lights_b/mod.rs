//! Lighting group B: Corsair headsets, Glorious wireless mice, HyperX mice, and
//! smaller vendors.

mod corsair_bragi;
mod corsair_headset;
mod glorious;
mod hyperx_mouse;
mod lanq;
mod layouts;
mod lg;
mod madlions;
mod mountain;
mod msi;
mod nuvoton;
mod nzxt_keyboard;
mod oem;
mod patriot;
mod prism;
mod rama;
mod vertagear;

use crate::hid::{Driver, Proto, open_io};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_fan::FanBackend;
use foxden_hid::registry::{Detector, Info, Open};
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    [
        corsair_bragi::DETECTORS,
        corsair_headset::DETECTORS,
        glorious::DETECTORS,
        hyperx_mouse::DETECTORS,
        lanq::DETECTORS,
        lg::DETECTORS,
        madlions::DETECTORS,
        mountain::DETECTORS,
        msi::DETECTORS,
        nuvoton::DETECTORS,
        nzxt_keyboard::DETECTORS,
        oem::DETECTORS,
        patriot::DETECTORS,
        prism::DETECTORS,
        rama::DETECTORS,
        vertagear::DETECTORS,
    ]
    .iter()
    .flat_map(|d| d.iter())
    .collect()
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn other_ids() -> Vec<(u16, u16, &'static str)> {
    Vec::new()
}

pub fn probe_other() -> Vec<Box<dyn Backend>> {
    Vec::new()
}

pub fn fan_backends(_api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    Vec::new()
}

const KEYBOARD: [f32; 4] = [0.10, 0.40, 0.60, 0.25];
const MOUSE: [f32; 4] = [0.76, 0.48, 0.08, 0.12];
const HEADSET: [f32; 4] = [0.80, 0.10, 0.10, 0.15];
const STRIP: [f32; 4] = [0.05, 0.85, 0.90, 0.08];
const MONITOR: [f32; 4] = [0.10, 0.05, 0.80, 0.12];

/// A detector on any mix of interface, usage page and usage.
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

/// LEDs at `[column, row]` cells of a `size` grid, as the plugins place them.
fn placed(
    info: &Info,
    family: &str,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    size: [u32; 2],
    pos: &[[u8; 2]],
) -> Device {
    let mut dev = Device::grid(&info.device_id(family), name, kind, rect, size[0], size[1]);
    dev.leds = pos
        .iter()
        .map(|p| {
            [
                (p[0] as f32 + 0.5) / size[0] as f32,
                (p[1] as f32 + 0.5) / size[1] as f32,
            ]
        })
        .collect();
    dev
}

/// A single row of `n` LEDs.
fn row(
    info: &Info,
    family: &str,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    n: usize,
) -> Device {
    Device::grid(&info.device_id(family), name, kind, rect, n as u32, 1)
}

fn boxed<P: Proto + 'static>(
    api: &HidApi,
    info: &Info,
    family: &'static str,
    dev: Device,
    proto: P,
) -> Result<Box<dyn Backend>, String> {
    Ok(Driver::new(open_io(api, info)?, dev, family, proto).boxed())
}

/// Colours placed at firmware indexes; `len` slots, unlisted ones black.
fn by_index(map: &[u8], colors: &[Color], len: usize) -> Vec<Color> {
    let mut out = vec![Color(0, 0, 0); len];
    for (&i, &c) in map.iter().zip(colors) {
        if (i as usize) < len {
            out[i as usize] = c;
        }
    }
    out
}

#[cfg(test)]
mod rec {
    use crate::hid::Io;

    /// What a driver sent, in order.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Op {
        Write(Vec<u8>),
        Feature(Vec<u8>),
        GetFeature(u8),
        Read,
    }

    /// An `Io` that records every call and answers reads from a queue.
    #[derive(Default)]
    pub struct Rec {
        pub ops: Vec<Op>,
        pub replies: Vec<Vec<u8>>,
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.ops.push(Op::Write(crate::hid::padded(data, len)));
            Ok(())
        }
        fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.ops.push(Op::Feature(crate::hid::padded(data, len)));
            Ok(())
        }
        fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String> {
            self.ops.push(Op::GetFeature(first));
            Ok(if self.replies.is_empty() {
                vec![0; len]
            } else {
                self.replies.remove(0)
            })
        }
        fn read(&mut self, _len: usize, _timeout_ms: i32) -> Result<Vec<u8>, String> {
            self.ops.push(Op::Read);
            Ok(if self.replies.is_empty() {
                Vec::new()
            } else {
                self.replies.remove(0)
            })
        }
        fn sleep(&mut self, _ms: u64) {}
    }

    pub fn writes(r: &Rec) -> Vec<Vec<u8>> {
        r.ops
            .iter()
            .filter_map(|o| match o {
                Op::Write(b) => Some(b.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn features(r: &Rec) -> Vec<Vec<u8>> {
        r.ops
            .iter()
            .filter_map(|o| match o {
                Op::Feature(b) => Some(b.clone()),
                _ => None,
            })
            .collect()
    }
}
