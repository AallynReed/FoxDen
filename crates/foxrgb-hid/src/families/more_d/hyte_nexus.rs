//! Ported from OpenRGB Controllers/HYTENexusController (GPL-2.0-or-later).

use std::time::{Duration, Instant};

use super::common::*;
use super::serial::{Port, Serial};
use foxrgb_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "hyte-nexus";
const VID: u16 = 0x3402;
const Q60: u16 = 0x0400;
const NP50: u16 = 0x0901;
pub const DEVICES: &[(u16, u16, &str)] = &[
    (VID, Q60, "HYTE THICC Q60"),
    (VID, NP50, "HYTE Nexus Portal NP50"),
];
const BAUD: u32 = 2_000_000;
const INFO_EVERY: Duration = Duration::from_millis(2500);

#[derive(Clone, Copy, Default)]
struct Channel {
    nexus: bool,
    logo: usize,
    lcd: bool,
}

fn channels(pid: u16) -> Vec<Channel> {
    let nexus = Channel {
        nexus: true,
        ..Channel::default()
    };
    match pid {
        NP50 => vec![Channel { logo: 6, ..nexus }, nexus, nexus],
        Q60 => vec![
            nexus,
            nexus,
            Channel {
                lcd: true,
                ..Channel::default()
            },
            Channel {
                logo: 4,
                ..Channel::default()
            },
        ],
        _ => Vec::new(),
    }
}

/// A query: write, flush, wait 50 ms, read the answer, drop what is left.
fn ask(io: &mut dyn Io, command: [u8; 4], reply: &mut [u8]) -> Result<(), String> {
    io.write(&command)?;
    io.flush(false, true);
    io.sleep(50);
    io.read(reply, 0)?;
    io.flush(true, false);
    Ok(())
}

fn device_info(io: &mut dyn Io) -> Result<(), String> {
    ask(io, [0xFF, 0xCC, 0x01, 0x00], &mut [0; 14])
}

/// The constructor's queries; returns the LED count of every channel.
pub fn probe_channels(io: &mut dyn Io, pid: u16) -> Result<Vec<usize>, String> {
    device_info(io)?;
    ask(io, [0xFF, 0xDD, 0x02, 0x00], &mut [0; 7])?;
    let mut counts = Vec::new();
    for (i, ch) in channels(pid).into_iter().enumerate() {
        let mut leds = 0;
        if ch.nexus {
            let mut rx = [0u8; 240];
            ask(io, [0xFF, 0xCC, 0x01, i as u8 + 1], &mut rx)?;
            leds = rx
                .chunks(12)
                .take(19)
                .filter(|d| d[2] > 0)
                .map(|d| d[5] as usize)
                .sum();
        }
        counts.push(leds + ch.logo + if ch.lcd { 42 } else { 0 });
    }
    Ok(counts)
}

/// LEDStreaming for one channel, colours in GRB order.
pub fn stream_packet(pid: u16, channel: u8, colors: &[Color]) -> Vec<u8> {
    let n = colors.len();
    let mut len = n * 3 + 7;
    if pid == Q60 && channel == 3 {
        len = len.max(90);
    }
    if pid == NP50 && channel == 2 {
        len = len.max(750);
    }
    let mut b = vec![0u8; len];
    b[..6].copy_from_slice(&[0xFF, 0xEE, 0x01, channel + 1, (n >> 8) as u8, n as u8]);
    for (i, c) in colors.iter().enumerate() {
        b[7 + i * 3..10 + i * 3].copy_from_slice(&[c.1, c.0, c.2]);
    }
    b
}

pub struct Nexus {
    pid: u16,
    counts: Vec<usize>,
    info_at: Instant,
}

impl Nexus {
    /// The keepalive thread's device info query, every 2.5 s whatever else is sent.
    fn keepalive(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.info_at.elapsed() > INFO_EVERY {
            device_info(io)?;
            self.info_at = Instant::now();
        }
        Ok(())
    }
}

impl Proto for Nexus {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for (channel, &n) in self.counts.iter().enumerate() {
            io.write(&stream_packet(self.pid, channel as u8, &colors[at..at + n]))?;
            io.flush(false, true);
            io.flush(true, false);
            at += n;
        }
        self.keepalive(io)
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    fn refresh(&mut self, io: &mut dyn Io, _colors: &[Color]) -> Result<(), String> {
        self.keepalive(io)
    }
}

pub fn open(port: &Port, name: &'static str) -> Option<Box<dyn Backend>> {
    let mut io = Serial::open(&port.name, BAUD, false).ok()?;
    let counts = probe_channels(&mut io, port.pid).ok()?;
    let dev = rows(
        named_id(FAMILY, &port.name),
        name,
        DeviceKind::Strip,
        &counts,
    );
    let nexus = Nexus {
        pid: port.pid,
        counts,
        info_at: Instant::now(),
    };
    Some(boxed(Box::new(io), dev, FAMILY, nexus))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn asked(cmd: [u8; 4], n: usize) -> Vec<Op> {
        vec![
            w(cmd.to_vec()),
            Op::Flush(false, true),
            Op::Sleep(50),
            Op::R(n),
            Op::Flush(true, false),
        ]
    }

    #[test]
    fn np50_channels() {
        let mut ch1 = vec![0u8; 240];
        ch1[2..6].copy_from_slice(&[1, 3, 1, 24]);
        ch1[12 + 2..12 + 6].copy_from_slice(&[1, 4, 1, 16]);
        let mut counts = Vec::new();
        let ops = record(vec![vec![0; 14], vec![0; 7], ch1], |io| {
            counts = probe_channels(io, NP50).unwrap()
        });
        assert_eq!(counts, vec![46, 0, 0]);
        let mut want = asked([0xFF, 0xCC, 1, 0], 14);
        want.extend(asked([0xFF, 0xDD, 2, 0], 7));
        for ch in 1..=3 {
            want.extend(asked([0xFF, 0xCC, 1, ch], 240));
        }
        assert_eq!(ops, want);
    }

    #[test]
    fn q60_streams_with_padding() {
        let nexus = Nexus {
            pid: Q60,
            counts: vec![1, 0, 42, 4],
            info_at: Instant::now(),
        };
        let mut a = vec![Color::BLACK; 47];
        a[0] = Color(1, 2, 3);
        a[46] = Color(4, 5, 6);
        let [first, _, _] = pushes(nexus, 47, vec![], &a, &[]);
        assert_eq!(first[0], w(vec![0xFF, 0xEE, 1, 1, 0, 1, 0, 2, 1, 3]));
        assert_eq!(first[1], Op::Flush(false, true));
        assert_eq!(first[2], Op::Flush(true, false));
        assert_eq!(first[3], w(vec![0xFF, 0xEE, 1, 2, 0, 0, 0]));
        assert_eq!(first[6], w(stream_packet(Q60, 2, &[Color::BLACK; 42])));
        let mut logo = pad(&[0xFF, 0xEE, 1, 4, 0, 4, 0], 90);
        logo[16..19].copy_from_slice(&[5, 4, 6]);
        assert_eq!(first[9], w(logo));
        assert_eq!(first.len(), 12);
        assert_eq!(stream_packet(NP50, 2, &[]).len(), 750);
    }
}
