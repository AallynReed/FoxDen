//! Flydigi Wireless Charger Dock 2 Pro (protocol learned from SignalRGB's Flydigi_Dock_2_Pro.js).

use super::super::common::{SMALL_RECT, placed};
use crate::hid::{Driver, Io, Proto, drain, open_io};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const LEN: usize = 65;
/// The dock takes 162 colours, after a zero byte and a brightness byte.
const POSITIONS: usize = 162;
const BRIGHTNESS: u8 = 100;
const CHUNK: usize = 50;
const PACKETS: u8 = 10;
const START: u8 = 0x16;
const DATA: u8 = 0x17;
/// Each step waits for its acknowledgement, up to 16 reads of 20 ms.
const ACK_READS: usize = 16;
const ACK_MS: i32 = 20;

/// LED grid: nonzero cells are LEDs, odd rows shift half a cell right.
const MAP: [[u8; 16]; 16] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 1, 1, 2, 3, 4, 0],
    [1, 1, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 1, 1, 0],
    [2, 1, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 1, 1, 1],
    [2, 1, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 1, 1, 0],
    [0, 1, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 1, 1, 0],
    [0, 1, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 1, 0, 0],
    [0, 0, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 1, 0, 0],
    [0, 0, 1, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 0, 0, 0],
    [0, 0, 0, 4, 5, 6, 7, 8, 9, 1, 1, 1, 1, 0, 0, 0],
    [0, 0, 0, 4, 5, 6, 7, 8, 9, 1, 1, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 5, 6, 7, 8, 9, 1, 1, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 5, 6, 7, 8, 9, 1, 1, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 6, 7, 8, 9, 1, 1, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 6, 7, 8, 9, 1, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 7, 8, 9, 1, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 7, 8, 9, 0, 0, 0, 0, 0, 0, 0],
];

fn leds() -> Vec<[f32; 2]> {
    let mut v = Vec::new();
    for (y, row) in MAP.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            if *cell != 0 {
                let lx = x * 2 + y % 2;
                v.push([(lx as f32 + 0.5) / 32.0, (y as f32 + 0.5) / 16.0]);
            }
        }
    }
    v.truncate(POSITIONS);
    v
}

/// `[0, 0x5A, 0xA5, cmd, body..., 1 + byte sum]`.
fn report(body: &[u8]) -> Vec<u8> {
    let mut p = vec![0x00, 0x5A, 0xA5];
    p.extend_from_slice(body);
    let sum = p[1..].iter().fold(1u8, |a, b| a.wrapping_add(*b));
    p.push(sum);
    p
}

pub fn start_report() -> Vec<u8> {
    report(&[START, 0x06, 0x00, 0x00, 0x00, PACKETS])
}

pub fn data_report(seq: u8, chunk: &[u8]) -> Vec<u8> {
    let mut body = vec![
        DATA,
        chunk.len() as u8 + 7,
        0x00,
        PACKETS,
        0x00,
        seq,
        chunk.len() as u8,
    ];
    body.extend_from_slice(chunk);
    report(&body)
}

pub fn payload(colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x00, BRIGHTNESS];
    p.extend(colors.iter().take(POSITIONS).flat_map(|c| [c.0, c.1, c.2]));
    p.resize(2 + POSITIONS * 3, 0);
    p
}

/// An acknowledgement starts `0x5A 0xA5 cmd`, after an optional zero byte; data acks echo the sequence.
pub fn is_ack(r: &[u8], cmd: u8, seq: Option<u8>) -> bool {
    let r = if r.first() == Some(&0) { &r[1..] } else { r };
    r.len() >= 8
        && r[..3] == [0x5A, 0xA5, cmd]
        && seq.is_none_or(|s| r[4..8] == [0x00, PACKETS, 0x00, s])
}

fn ack(io: &mut dyn Io, cmd: u8, seq: Option<u8>) -> Result<(), String> {
    io.sleep(1);
    for _ in 0..ACK_READS {
        let r = io.read(LEN, ACK_MS)?;
        if is_ack(&r, cmd, seq) {
            return Ok(());
        }
    }
    Err(format!("dock did not acknowledge {cmd:#04x}"))
}

struct Dock;

impl Proto for Dock {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        drain(io, LEN);
        io.write(&start_report(), LEN)?;
        ack(io, START, None)?;
        for (seq, ch) in payload(colors).chunks(CHUNK).enumerate() {
            io.write(&data_report(seq as u8, ch), LEN)?;
            ack(io, DATA, Some(seq as u8))?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = placed(
        &info.device_id("flydigi-dock"),
        d.name,
        DeviceKind::Other,
        SMALL_RECT,
        [32, 16],
        leds(),
    );
    Ok(Driver::new(open_io(api, info)?, dev, "flydigi-dock", Dock).boxed())
}

pub static DETECTORS: &[Detector] = &[Detector {
    name: "Flydigi Wireless Charger Dock 2 Pro",
    vid: 0x37D7,
    pid: 0x6001,
    interface: None,
    usage_page: Some(0xFFA0),
    usage: Some(0x0001),
    open,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_and_acks() {
        assert_eq!(leds().len(), POSITIONS);
        let s = start_report();
        assert_eq!(
            s,
            vec![
                0,
                0x5A,
                0xA5,
                0x16,
                6,
                0,
                0,
                0,
                10,
                (1u32 + 0x5A + 0xA5 + 0x16 + 6 + 10) as u8
            ]
        );
        let p = payload(&[Color(1, 2, 3)]);
        assert_eq!(p.len(), 488);
        assert_eq!(p.chunks(CHUNK).count(), 10);
        let d = data_report(9, &p[450..]);
        assert_eq!(d[3..10], [0x17, 45, 0, 10, 0, 9, 38]);
        assert!(is_ack(&[0x5A, 0xA5, 0x17, 0, 0, 10, 0, 9], DATA, Some(9)));
        assert!(is_ack(&[0, 0x5A, 0xA5, 0x16, 0, 0, 0, 0, 0], START, None));
        assert!(!is_ack(&[0x5A, 0xA5, 0x17, 0, 0, 10, 0, 8], DATA, Some(9)));
    }
}
