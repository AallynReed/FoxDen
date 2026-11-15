//! Ported from OpenRGB Controllers/ZalmanZSyncController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "zalman-zsync";
const CHANNELS: usize = 8;
/// OpenRGB leaves channel sizes to the user; FoxRGB drives each at its 40-LED maximum.
const CHANNEL_LEDS: usize = 40;
const FIRMWARE: u8 = 0x02;
const DIRECT: u8 = 0x32;
const COMMIT: u8 = 0x33;
const PORT_STATE: u8 = 0x38;
const SOFTWARE: u8 = 0x02;
/// OpenRGB blocks on each 17-byte reply; this bounds the wait.
const REPLY_MS: i32 = 1000;

fn packet(head: &[u8]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

fn exchange(io: &mut dyn Io, p: &[u8; 65]) -> Result<(), String> {
    io.write(p)?;
    let mut reply = [0u8; 17];
    io.read(&mut reply, REPLY_MS)?;
    Ok(())
}

/// SetChannelLEDs: software port state, each colour plane in blocks of 50, then commit.
pub fn channel_packets(channel: u8, colors: &[Color]) -> Vec<[u8; 65]> {
    let mut out = vec![packet(&[PORT_STATE, channel, SOFTWARE])];
    for (block, chunk) in colors.chunks(50).enumerate() {
        let start = (block * 50) as u8;
        for plane in 0..3u8 {
            let mut p = packet(&[DIRECT, channel, start, chunk.len() as u8, plane]);
            for (i, c) in chunk.iter().enumerate() {
                p[6 + i] = rgb(*c)[plane as usize];
            }
            out.push(p);
        }
    }
    out.push(packet(&[COMMIT, 0xFF]));
    out
}

struct Zalman;

impl Proto for Zalman {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        exchange(io, &packet(&[FIRMWARE]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (ch, zone) in colors.chunks(CHANNEL_LEDS).enumerate() {
            for p in channel_packets(ch as u8, zone) {
                exchange(io, &p)?;
            }
        }
        Ok(())
    }
    // the keepalive thread commits again after a second without one
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
}

fn device(info: &Info, d: &Detector) -> Device {
    let kind = DeviceKind::Strip;
    let mut dev = Device::grid(
        &info.device_id(FAMILY),
        d.name,
        kind,
        [0.06, 0.40, 0.60, 0.16],
        CHANNEL_LEDS as u32,
        CHANNELS as u32,
    );
    dev.leds = (0..CHANNELS)
        .flat_map(|r| {
            (0..CHANNEL_LEDS).map(move |i| {
                [
                    (i as f32 + 0.5) / CHANNEL_LEDS as f32,
                    (r as f32 + 0.5) / CHANNELS as f32,
                ]
            })
        })
        .collect();
    dev
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    boxed(api, info, FAMILY, device(info, d), Zalman)
}

pub static DETECTORS: &[Detector] = &[det("Zalman Z Sync", 0x1C57, 0x7ED0, ANY, ANY, ANY, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetChannelLEDs: SendPortState, SendDirect per colour plane, SendCommit
    #[test]
    fn channel_sequence() {
        let p = channel_packets(3, &[Color(1, 2, 3), Color(4, 5, 6)]);
        let b = |h: &[u8]| {
            let mut v = vec![0u8];
            v.extend(pad(h, 64));
            v
        };
        assert_eq!(p.len(), 5);
        assert_eq!(p[0].to_vec(), b(&[0x38, 3, 2]));
        assert_eq!(p[1].to_vec(), b(&[0x32, 3, 0, 2, 0, 1, 4]));
        assert_eq!(p[2].to_vec(), b(&[0x32, 3, 0, 2, 1, 2, 5]));
        assert_eq!(p[3].to_vec(), b(&[0x32, 3, 0, 2, 2, 3, 6]));
        assert_eq!(p[4].to_vec(), b(&[0x33, 0xFF]));
        let long = channel_packets(0, &[Color(7, 7, 7); 60]);
        assert_eq!(long.len(), 1 + 6 + 1);
        assert_eq!(long[4][1..6], [0x32, 0, 50, 10, 0]);
    }

    // SendFirmwareRequest from the constructor, then every exchange reads a reply
    #[test]
    fn firmware_then_reads() {
        let a = vec![Color::BLACK; CHANNELS * CHANNEL_LEDS];
        let [first, _, _] = pushes(Zalman, a.len(), vec![], &a, &[]);
        assert_eq!(first[0], w(packet(&[0x02]).to_vec()));
        assert_eq!(first[1], Op::Read);
        assert_eq!(first.len(), 2 + CHANNELS * 5 * 2);
    }
}
