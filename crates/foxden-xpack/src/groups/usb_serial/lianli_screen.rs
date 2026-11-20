//! Lian Li Universal Screen 8.8 LED frame over raw USB (protocol learned from SignalRGB's
//! Lian_Li_Universal_Screen_88_LED.js).

use super::link::open_bulk;
use crate::hid::{Driver, Io, Proto, rgb};
use foxden_core::{Backend, Color, Device, DeviceKind};

const FAMILY: &str = "lianli-screen88-led";
pub const NAME: &str = "Lian Li Universal Screen 8.8 LED";
pub const VID: u16 = 0x0416;
pub const PID: u16 = 0x8050;
const ENDPOINT: u8 = 0x01;
const SET_LEDS: u8 = 0x11;
const PACKET: usize = 64;
const PER_PACKET: usize = 20;
const LEDS: usize = 60;
const COLS: u32 = 23;
const ROWS: u32 = 9;

/// Wire order around the bezel on a 23 x 9 grid: top right to left, down, bottom, up, top again.
fn positions() -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = (0..14).rev().map(|x| (x, 0)).collect();
    v.extend((1..8).map(|y| (0, y)));
    v.extend((0..23).map(|x| (x, 8)));
    v.extend((1..8).rev().map(|y| (22, y)));
    v.extend((14..23).rev().map(|x| (x, 0)));
    v
}

pub fn packets(colors: &[Color]) -> Vec<Vec<u8>> {
    colors
        .chunks(PER_PACKET)
        .enumerate()
        .map(|(i, chunk)| {
            let mut p = vec![SET_LEDS, (i * PER_PACKET) as u8, 0, 0];
            p.extend(chunk.iter().flat_map(|c| rgb(*c)));
            p
        })
        .collect()
}

struct Screen;

impl Proto for Screen {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in packets(colors) {
            io.write(&p, PACKET)?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); LEDS]);
    }
}

fn device(id: &str) -> Device {
    let mut dev = Device::grid(
        id,
        NAME,
        DeviceKind::Other,
        [0.30, 0.10, 0.30, 0.12],
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
    dev
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    open_bulk(FAMILY, VID, PID, ENDPOINT)
        .into_iter()
        .map(|f| Driver::new(Box::new(f.io), device(&f.id), FAMILY, Screen).boxed())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezel_order() {
        let p = positions();
        assert_eq!(p.len(), LEDS);
        assert_eq!(p[0], (13, 0));
        assert_eq!(p[13], (0, 0));
        assert_eq!(p[14], (0, 1));
        assert_eq!(p[21], (0, 8));
        assert_eq!(p[43], (22, 8));
        assert_eq!(p[44], (22, 7));
        assert_eq!(p[51], (22, 0));
        assert_eq!(p[59], (14, 0));
    }

    #[test]
    fn three_packets() {
        let c: Vec<Color> = (0..60).map(|i| Color(i, 0, 1)).collect();
        let p = packets(&c);
        assert_eq!(p.len(), 3);
        assert_eq!(&p[1][..7], &[0x11, 20, 0, 0, 20, 0, 1]);
        assert_eq!(p[2].len(), 4 + 60);
    }
}
