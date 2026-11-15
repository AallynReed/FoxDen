//! Cooler Master Atmos II Pixel LED matrix (protocol learned from SignalRGB's
//! CoolerMaster_Atmos_II_Pixel_LCD.js).

use crate::hid::{Driver, Io, Proto, open_io, rgb};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "coolermaster-atmos2-pixel";
const COLS: u32 = 26;
const ROWS: u32 = 28;
const REPORT: usize = 65;
const PACKETS: usize = 28;
const PAYLOAD: usize = 60;
const SET_PIXELS: [u8; 2] = [0x80, 0xDD];
/// First and last lit column of each row of the round matrix; rows run serpentine from the top.
#[rustfmt::skip]
const SPANS: [(u32, u32); ROWS as usize] = [
    (11, 14), (10, 15), (8, 17), (6, 19), (4, 21), (3, 22), (1, 24),
    (0, 25), (0, 25), (0, 25), (0, 25), (0, 25), (0, 25), (0, 25),
    (0, 25), (0, 25), (0, 25), (0, 25), (0, 25), (0, 25), (0, 25),
    (1, 24), (3, 22), (4, 21), (6, 19), (8, 17), (10, 15), (11, 14),
];

/// Wire order: even rows left to right, odd rows right to left.
pub fn positions() -> Vec<(u32, u32)> {
    SPANS
        .iter()
        .enumerate()
        .flat_map(|(y, &(a, b))| {
            let xs: Vec<u32> = if y % 2 == 0 {
                (a..=b).collect()
            } else {
                (a..=b).rev().collect()
            };
            xs.into_iter().map(move |x| (x, y as u32))
        })
        .collect()
}

/// 28 reports: report id 0, command, packet index (big endian), 60 colour bytes zero padded.
pub fn reports(colors: &[Color]) -> Vec<Vec<u8>> {
    let stream: Vec<u8> = colors.iter().flat_map(|c| rgb(*c)).collect();
    (0..PACKETS)
        .map(|i| {
            let mut r = vec![0x00, SET_PIXELS[0], SET_PIXELS[1]];
            r.extend((i as u16).to_be_bytes());
            r.extend((0..PAYLOAD).map(|j| stream.get(i * PAYLOAD + j).copied().unwrap_or(0)));
            r
        })
        .collect()
}

struct Atmos(usize);

impl Proto for Atmos {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for r in reports(colors) {
            io.write(&r, REPORT)?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    // Shutdown paints the default shutdown colour, black
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &vec![Color(0, 0, 0); self.0]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut dev = Device::grid(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Other,
        [0.40, 0.30, 0.14, 0.15],
        COLS,
        ROWS,
    );
    dev.leds = positions()
        .into_iter()
        .map(|(x, y)| {
            [
                (x as f32 + 0.5) / COLS as f32,
                (y as f32 + 0.5) / ROWS as f32,
            ]
        })
        .collect();
    let n = dev.leds.len();
    Ok(Driver::new(open_io(api, info)?, dev, FAMILY, Atmos(n)).boxed())
}

pub static DETECTOR: Detector = Detector {
    name: "Cooler Master Atmos II Pixel",
    vid: 0x2516,
    pid: 0x021C,
    interface: Some(1),
    usage_page: Some(0xFF01),
    usage: Some(0x0001),
    open,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_order() {
        let p = positions();
        assert_eq!(p.len(), 556);
        assert_eq!(&p[..5], &[(11, 0), (12, 0), (13, 0), (14, 0), (15, 1)]);
        assert_eq!(p[121], (0, 7));
        assert_eq!(p[555], (11, 27));
    }

    #[test]
    fn report_split() {
        let c: Vec<Color> = (0..556).map(|i| Color(i as u8, 1, 2)).collect();
        let r = reports(&c);
        assert_eq!(r.len(), 28);
        assert_eq!(&r[0][..8], &[0, 0x80, 0xDD, 0, 0, 0, 1, 2]);
        assert_eq!(&r[27][..5], &[0, 0x80, 0xDD, 0, 27]);
        assert_eq!(r[27].len(), 65);
        // 556 * 3 = 1668 colour bytes, the last 12 of 1680 are padding
        assert!(r[27][5 + 48..].iter().all(|&b| b == 0));
        assert_eq!(r[27][5 + 47], 2);
    }
}
