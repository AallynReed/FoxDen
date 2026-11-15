//! Layout helpers shared by the lights_a drivers.

use foxrgb_core::{Color, Device, DeviceKind};
use foxrgb_hid::registry::{Detector, Open};

/// A device whose LEDs sit on whole cells of a `size` grid, given as `[x, y]` per LED.
pub fn keyed(
    id: &str,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    size: [u32; 2],
    cells: &[[u8; 2]],
) -> Device {
    let [w, h] = [size[0].max(1), size[1].max(1)];
    let mut dev = Device::grid(id, name, kind, rect, w, h);
    dev.leds = cells
        .iter()
        .map(|[x, y]| [(*x as f32 + 0.5) / w as f32, (*y as f32 + 0.5) / h as f32])
        .collect();
    dev
}

/// A device with explicit LED positions.
pub fn placed(
    id: &str,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
    grid: [u32; 2],
    leds: Vec<[f32; 2]>,
) -> Device {
    let mut dev = Device::grid(id, name, kind, rect, grid[0].max(1), grid[1].max(1));
    dev.leds = leds;
    dev
}

/// A detector on one interface, usage page and usage.
pub const fn full(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: i32,
    page: u16,
    usage: u16,
    open: Open,
) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: Some(interface),
        usage_page: Some(page),
        usage: Some(usage),
        open,
    }
}

/// ASUS Aura peripheral direct packet: `[id, 0xC0, 0x81, count, 0, (index, r, g, b) x 15]`, 65 bytes.
pub fn asus_direct(report: u8, count: u8, slots: &[(u8, Color)]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[..4].copy_from_slice(&[report, 0xC0, 0x81, count]);
    for (i, (idx, c)) in slots.iter().take(15).enumerate() {
        b[5 + i * 4..9 + i * 4].copy_from_slice(&[*idx, c.0, c.1, c.2]);
    }
    b
}

/// ASUS peripheral "hardware mode" restore sent on shutdown, then the save command.
pub const ASUS_RESTORE: [u8; 39] = [
    0x00, 0x51, 0x2C, 0x04, 0x00, 0x48, 0x64, 0x00, 0x00, 0x02, 0x07, 0x0E, 0xF5, 0x00, 0xFF, 0x1D,
    0x00, 0x06, 0xFF, 0x2B, 0x00, 0xFA, 0xFF, 0x39, 0x01, 0xFF, 0x00, 0x48, 0xFF, 0xF6, 0x00, 0x56,
    0xFF, 0x78, 0x07, 0x64, 0xFF, 0x00, 0x0D,
];
pub const ASUS_SAVE: [u8; 3] = [0x00, 0x50, 0x55];

/// Default canvas rectangles per device kind.
pub const KEYBOARD_RECT: [f32; 4] = [0.05, 0.55, 0.60, 0.25];
pub const MOUSE_RECT: [f32; 4] = [0.70, 0.60, 0.10, 0.15];
pub const SMALL_RECT: [f32; 4] = [0.40, 0.10, 0.20, 0.20];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_cells_are_centred() {
        let d = keyed(
            "d",
            "D",
            DeviceKind::Keyboard,
            KEYBOARD_RECT,
            [4, 2],
            &[[0, 0], [3, 1]],
        );
        assert_eq!(d.leds, vec![[0.125, 0.25], [0.875, 0.75]]);
    }
}
