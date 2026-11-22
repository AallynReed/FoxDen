//! Antec Vortex View 360 AIO.

use super::super::common::{SMALL_RECT, keyed};
use crate::fan::{Channel, FanHub, backend};
use crate::hid::{Driver, Io, Proto, drain, read_until};
use foxden_core::{Backend, Color, DeviceKind};
use foxden_fan::FanBackend;
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;
use std::time::{Duration, Instant};

pub const VID: u16 = 0x2433;
pub const PID: u16 = 0xA100;
const LEN: usize = 65;
const PAGE: u16 = 0xFF00;
const CELLS: [[u8; 2]; 18] = [
    [0, 3],
    [1, 3],
    [2, 3],
    [3, 3],
    [4, 3],
    [4, 2],
    [4, 1],
    [4, 0],
    [3, 0],
    [2, 0],
    [1, 0],
    [0, 0],
    [0, 1],
    [0, 2],
    [1, 2],
    [3, 2],
    [3, 1],
    [1, 1],
];
/// The panel drops off the bus about 40 s after the last heartbeat; the plugin beats every second.
const HEARTBEAT_MS: u64 = 1000;
const LOW_LEDS: usize = 14;

/// `[0, cmd hi, cmd lo, body...]` with the byte sum (checksum slot zero) at `slot + 1`.
pub fn packet(cmd: [u8; 2], body: &[u8], slot: usize) -> Vec<u8> {
    let mut p = vec![0u8; LEN];
    p[1..3].copy_from_slice(&cmd);
    p[3..3 + body.len()].copy_from_slice(body);
    p[slot + 1] = p.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    p
}

/// LEDs 0-13 in `0x50 0x05`, 14-17 in `0x50 0x06`, each as `[g, r, b, 0]`.
pub fn colour_packets(colors: &[Color]) -> [Vec<u8>; 2] {
    let grb = |cs: &[Color]| -> Vec<u8> { cs.iter().flat_map(|c| [c.1, c.0, c.2, 0]).collect() };
    let (lo, hi) = colors.split_at(LOW_LEDS.min(colors.len()));
    let mut a = vec![0x38];
    a.extend(grb(lo));
    let mut b = vec![0x10];
    b.extend(grb(hi));
    [packet([0x50, 0x05], &a, 59), packet([0x50, 0x06], &b, 19)]
}

fn heartbeat() -> Vec<u8> {
    packet([0x20, 0x18], &[0x01, 0x00], 4)
}

struct Antec {
    beat: Instant,
}

impl Proto for Antec {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&packet([0x00, 0x33], &[0x01, 0x01], 4), LEN)?;
        io.write(&packet([0x50, 0x0A], &[0x01, 100], 4), LEN)?;
        io.write(&heartbeat(), LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in colour_packets(colors) {
            io.write(&p, LEN)?;
        }
        if self.beat.elapsed() >= Duration::from_millis(HEARTBEAT_MS) {
            io.write(&heartbeat(), LEN)?;
            self.beat = Instant::now();
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(HEARTBEAT_MS)
    }
}

pub fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = keyed(
        &info.device_id("antec-vortex"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [5, 4],
        &CELLS,
    );
    let io = Box::new(info.open(api)?);
    Ok(Driver::new(
        io,
        dev,
        "antec-vortex",
        Antec {
            beat: Instant::now(),
        },
    )
    .boxed())
}

/// Coolant temperature from a `0x30 0x22` reply: confirmed at 4, probe present at 7, whole and tenths at 8 and 9.
pub fn coolant(reply: &[u8]) -> Option<f32> {
    if reply.len() < 10 || reply[4] != 1 || reply[7] == 0 {
        return None;
    }
    Some(reply[8] as f32 + reply[9] as f32 / 10.0)
}

/// The AIO's coolant sensor; it has no fan or pump control.
struct Sensor {
    io: hidapi::HidDevice,
    id: String,
}

impl FanHub for Sensor {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        "Antec Vortex View 360".into()
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
        let io: &mut dyn Io = &mut self.io;
        drain(io, LEN);
        io.write(&packet([0x30, 0x22], &[0x02, 0x00, 0x01], 5), LEN)
            .ok()?;
        // hidapi drops the 0 report id the plugin's offsets count; replies to lighting writes are skipped
        let js = |r: &[u8]| [&[0u8][..], r].concat();
        let r = read_until(io, LEN, 100, |r| coolant(&js(r)).is_some()).ok()?;
        coolant(&js(&r)).filter(|t| *t != 0.0)
    }
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for raw in foxden_hid::device_list(api).iter() {
        let info = Info::from_hidapi(raw);
        if info.vid != VID
            || info.pid != PID
            || info.interface != 0
            || info.usage_page != PAGE
            || seen.contains(&info.key())
        {
            continue;
        }
        if let Ok(io) = info.open(api) {
            seen.push(info.key());
            let id = format!(
                "fan:{}",
                info.device_id("antec-vortex").trim_start_matches("hid:")
            );
            out.push(backend(Sensor { io, id }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksummed_packets() {
        let h = heartbeat();
        assert_eq!(h[..6], [0, 0x20, 0x18, 1, 0, 0x39]);
        let [lo, hi] = colour_packets(&[Color(1, 2, 3); 18]);
        assert_eq!(lo[..8], [0, 0x50, 0x05, 0x38, 2, 1, 3, 0]);
        let sum = lo
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 60)
            .fold(0u8, |a, (_, b)| a.wrapping_add(*b));
        assert_eq!(lo[60], sum);
        assert_eq!(hi[3], 0x10);
        assert_eq!(
            hi[20],
            hi.iter().take(20).fold(0u8, |a, b| a.wrapping_add(*b))
        );
    }

    #[test]
    fn coolant_reply() {
        assert_eq!(coolant(&[0, 0, 0, 0, 1, 0, 0, 1, 31, 5]), Some(31.5));
        assert_eq!(coolant(&[0, 0, 0, 0, 1, 0, 0, 0, 31, 5]), None);
    }
}
