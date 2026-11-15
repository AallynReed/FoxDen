//! ASUS ROG Swift monitors (protocol learned from SignalRGB's ASUS_Monitor_Controller.js).

use super::common::{SMALL_RECT, full, keyed};
use crate::hid::{Driver, Io, Proto, open_io};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x0B05;

/// Monitor register write: feature report 3, `[3, 2, 0xA1, 0x80, register, value, 0, 0]`.
pub fn monitor_reg(register: u8, value: u8) -> [u8; 8] {
    [0x03, 0x02, 0xA1, 0x80, register, value, 0x00, 0x00]
}

/// Every frame: registers 0x20, 0x30, 0xA0 set to 1, then per LED red, blue, green at 3*i.
pub fn monitor_frame(colors: &[Color]) -> Vec<[u8; 8]> {
    let mut v = vec![
        monitor_reg(0x20, 1),
        monitor_reg(0x30, 1),
        monitor_reg(0xA0, 1),
    ];
    for (i, c) in colors.iter().enumerate() {
        let r = (i * 3) as u8;
        v.extend([
            monitor_reg(r, c.0),
            monitor_reg(r + 1, c.2),
            monitor_reg(r + 2, c.1),
        ]);
    }
    v
}

struct Monitor;

impl Proto for Monitor {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for r in monitor_frame(colors) {
            io.feature(&r, 8)?;
        }
        Ok(())
    }
}

/// Product id, grid size and LED cells.
type Model = (u16, [u32; 2], &'static [[u8; 2]]);

const MONITORS: &[Model] = &[
    (0x186E, [3, 1], &[[0, 0], [1, 0], [2, 0]]),
    (0x1931, [1, 1], &[[0, 0]]),
    (0x1AC5, [1, 1], &[[0, 0]]),
    (0x1941, [5, 3], &[[0, 0], [1, 0], [3, 0], [4, 0], [2, 2]]),
    (0x18D4, [5, 3], &[[0, 0], [1, 0], [3, 0], [4, 0], [2, 2]]),
];

fn open_monitor(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (_, size, cells) = MONITORS
        .iter()
        .find(|m| m.0 == info.pid)
        .ok_or("unknown ASUS monitor")?;
    let dev = keyed(
        &info.device_id("asus-monitor"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        *size,
        cells,
    );
    Ok(Driver::new(open_io(api, info)?, dev, "asus-monitor", Monitor).boxed())
}

static DETECTORS: &[Detector] = &[
    full("ASUS PG27UQ", VID, 0x186E, 0, 0xFFA0, 1, open_monitor),
    full("ASUS PG43UQ", VID, 0x1931, 0, 0xFFA0, 1, open_monitor),
    full("ASUS PG248QP", VID, 0x1AC5, 0, 0xFFA0, 1, open_monitor),
    full("ASUS PG32UQX", VID, 0x1941, 0, 0xFFA0, 1, open_monitor),
    full("ASUS PG35VQ", VID, 0x18D4, 0, 0xFFA0, 1, open_monitor),
];

pub fn detectors() -> Vec<&'static Detector> {
    DETECTORS.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_order_is_red_blue_green() {
        let f = monitor_frame(&[Color(1, 2, 3)]);
        assert_eq!(f[0], [3, 2, 0xA1, 0x80, 0x20, 1, 0, 0]);
        assert_eq!(f[4][4..6], [1, 3]);
        assert_eq!(f[5][4..6], [2, 2]);
    }
}
