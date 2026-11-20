//! LANQ PCDOCK monitor stand ARGB over USB serial (protocol learned from SignalRGB's
//! PCDOCK_Monitor_Stand.js).

use super::link::open_serial;
use crate::hid::{Driver, Io, Proto, rgb};
use foxden_core::{Backend, Color, Device, DeviceKind};

const FAMILY: &str = "pcdock-stand";
pub const NAME: &str = "PCDOCK Monitor Stand";
pub const VID: u16 = 0x1C01;
pub const PID: u16 = 0x1002;
const BAUD: u32 = 115_200;
const MAGIC: &[u8] = b"MP4F";
const VERSION: u8 = 0x01;
const FRAME: u8 = 0x01;
const RELEASE: u8 = 0x02;
const HEADER_LEN: u16 = 20;
const CHANNELS: usize = 2;
/// The plugin leaves the strip length to the user; FoxDen drives each channel at its maximum.
const CHANNEL_LEDS: usize = 240;

/// Reflected CRC-32 (0xEDB88320, init and final xor 0xFFFFFFFF).
pub fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(0xFFFF_FFFFu32, |mut c, b| {
        c ^= *b as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ if c & 1 != 0 { 0xEDB8_8320 } else { 0 };
        }
        c
    })
}

/// Header (all fields little endian), payload, then the CRC-32 of both.
pub fn message(kind: u8, seq: u32, ch1: &[Color], ch2: &[Color]) -> Vec<u8> {
    let payload: Vec<u8> = ch1.iter().chain(ch2).flat_map(|c| rgb(*c)).collect();
    let mut m = MAGIC.to_vec();
    m.extend([VERSION, kind]);
    m.extend(HEADER_LEN.to_le_bytes());
    m.extend(seq.to_le_bytes());
    m.extend((payload.len() as u16).to_le_bytes());
    m.extend((ch1.len() as u16).to_le_bytes());
    m.extend((ch2.len() as u16).to_le_bytes());
    m.extend([0, 0]);
    m.extend(payload);
    let crc = crc32(&m);
    m.extend(crc.to_le_bytes());
    m
}

#[derive(Default)]
struct Stand {
    seq: u32,
}

impl Stand {
    fn next(&mut self) -> u32 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }
}

impl Proto for Stand {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let (a, b) = colors.split_at(CHANNEL_LEDS.min(colors.len()));
        let seq = self.next();
        io.write(&message(FRAME, seq, a, b), 0)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); CHANNELS * CHANNEL_LEDS]);
        let seq = self.next();
        let _ = io.write(&message(RELEASE, seq, &[], &[]), 0);
    }
}

fn device(id: &str) -> Device {
    Device::grid(
        id,
        NAME,
        DeviceKind::Strip,
        [0.06, 0.02, 0.88, 0.06],
        CHANNEL_LEDS as u32,
        CHANNELS as u32,
    )
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    open_serial(FAMILY, VID, &[PID], BAUD)
        .into_iter()
        .map(|p| Driver::new(Box::new(p.io), device(&p.id), FAMILY, Stand::default()).boxed())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn frame_header() {
        let m = message(
            FRAME,
            0x0102_0304,
            &[Color(1, 2, 3)],
            &[Color(4, 5, 6), Color(7, 8, 9)],
        );
        assert_eq!(m.len(), 20 + 9 + 4);
        assert_eq!(
            &m[..20],
            &[
                b'M', b'P', b'4', b'F', 1, 1, 20, 0, 4, 3, 2, 1, 9, 0, 1, 0, 2, 0, 0, 0
            ]
        );
        assert_eq!(&m[20..29], &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(m[29..].to_vec(), crc32(&m[..29]).to_le_bytes().to_vec());
    }

    #[test]
    fn release_is_bare_header() {
        let m = message(RELEASE, 5, &[], &[]);
        assert_eq!(m.len(), 24);
        assert_eq!(m[5], 0x02);
        assert_eq!(&m[12..18], &[0; 6]);
    }
}
