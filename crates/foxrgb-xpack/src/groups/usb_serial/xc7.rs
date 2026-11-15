//! Corsair XC7 RGB Elite LCD ring and coolant probe (protocol learned from SignalRGB's
//! Corsair_XC7_LCD.js).

use crate::fan::{Channel, FanHub, backend};
use crate::hid::{Driver, Io, Proto, on_i, open_io, rgb};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::{Bus, Detector, Info};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

const FAMILY: &str = "corsair-xc7";
pub const NAME: &str = "Corsair XC7 RGB Elite LCD";
pub const VID: u16 = 0x1B1C;
pub const PID: u16 = 0x0C42;
const RING_REPORT: usize = 1024;
const RING: [u8; 3] = [0x02, 0x07, 0x1F];
const FEATURE: usize = 32;
const SOFTWARE: &[u8] = &[0x03, 0x1D, 0x01, 0x00];
const HARDWARE: &[u8] = &[0x03, 0x1D, 0x00, 0x01, 0x04, 0x00, 0xA3];
const LIQUID: u8 = 0x18;
const POLL: Duration = Duration::from_millis(3000);
const GRID: u32 = 13;
/// Ring LEDs on a 13 x 13 grid, in wire order.
#[rustfmt::skip]
const RING_POS: [(u32, u32); 31] = [
    (9, 9), (8, 10), (7, 11), (6, 12), (6, 12), (5, 12), (4, 12), (4, 12), (3, 11), (2, 10),
    (1, 9), (0, 8), (0, 7), (0, 6), (0, 6), (0, 5), (0, 5), (0, 4), (1, 3), (2, 2),
    (3, 1), (4, 0), (4, 0), (5, 0), (6, 0), (6, 0), (7, 1), (8, 2), (9, 3), (11, 6),
    (12, 6),
];

pub fn ring_report(colors: &[Color]) -> Vec<u8> {
    let mut r = RING.to_vec();
    r.extend(colors.iter().flat_map(|c| rgb(*c)));
    r
}

/// Liquid temperature in a 0x18 feature reply: little endian tenths of a degree, 0 when absent.
pub fn liquid(reply: &[u8]) -> Option<f32> {
    let raw = u16::from_le_bytes([*reply.get(2)?, *reply.get(3)?]);
    (raw != 0).then(|| raw as f32 / 10.0)
}

struct Ring;

impl Proto for Ring {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(SOFTWARE, FEATURE)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&ring_report(colors), RING_REPORT)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = io.feature(HARDWARE, FEATURE);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut dev = Device::grid(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Other,
        [0.80, 0.40, 0.08, 0.14],
        GRID,
        GRID,
    );
    dev.leds = RING_POS
        .iter()
        .map(|&(x, y)| {
            [
                (x as f32 + 0.5) / GRID as f32,
                (y as f32 + 0.5) / GRID as f32,
            ]
        })
        .collect();
    Ok(Driver::new(open_io(api, info)?, dev, FAMILY, Ring).boxed())
}

pub static DETECTORS: [Detector; 2] = [
    on_i(NAME, VID, PID, 0, open),
    on_i(NAME, VID, PID, -1, open),
];

/// The coolant probe as a sensor-only hub; the block has no fan or pump header.
struct Probe {
    dev: HidDevice,
    id: String,
    last: Option<(Instant, Option<f32>)>,
}

impl FanHub for Probe {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        NAME.into()
    }
    fn channels(&self) -> Vec<Channel> {
        Vec::new()
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Liquid".into()]
    }
    fn rpm(&mut self, _channel: usize) -> Option<u32> {
        None
    }
    fn set_duty(&mut self, _channel: usize, _duty: f32) -> Result<(), String> {
        Err("no fan channels".into())
    }
    fn release(&mut self, _channel: usize) -> Result<(), String> {
        Ok(())
    }
    fn temperature(&mut self, _sensor: usize) -> Option<f32> {
        if let Some((at, t)) = self.last
            && at.elapsed() < POLL
        {
            return t;
        }
        let t = Io::get_feature(&mut self.dev, LIQUID, FEATURE)
            .ok()
            .and_then(|r| liquid(&r));
        self.last = Some((Instant::now(), t));
        t
    }
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for raw in foxrgb_hid::device_list(api).iter() {
        let info = Info::from_hidapi(raw);
        if info.vid != VID
            || info.pid != PID
            || info.bus != Bus::Usb
            || !matches!(info.interface, 0 | -1)
            || seen.contains(&info.key())
        {
            continue;
        }
        let Ok(dev) = info.open(api) else {
            continue;
        };
        seen.push(info.key());
        let id = info.device_id(FAMILY).replacen("hid:", "fan:", 1);
        out.push(backend(Probe {
            dev,
            id,
            last: None,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_layout() {
        let r = ring_report(&[Color(1, 2, 3); 31]);
        assert_eq!(r.len(), 3 + 93);
        assert_eq!(&r[..6], &[0x02, 0x07, 0x1F, 1, 2, 3]);
    }

    #[test]
    fn liquid_tenths() {
        assert_eq!(liquid(&[0x18, 0, 0x2C, 0x01]), Some(30.0));
        assert_eq!(liquid(&[0x18, 0, 0, 0]), None);
        assert_eq!(liquid(&[0x18]), None);
    }
}
