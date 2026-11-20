//! Ported from OpenRGB Controllers/NollieController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, any, device, on_i, open_with, rows, span};
use crate::registry::{Detector, Info};

const OS2_VID: u16 = 0x16D5;
const HS_VID: u16 = 0x3061;
const FS_VID: u16 = 0x16D2;
const NOLLIE32: u16 = 0x4714;
const NOLLIE16: u16 = 0x4716;
const NOLLIE8: u16 = 0x1F01;
const NOLLIE1: u16 = 0x1F11;
const NOLLIE28_12: u16 = 0x1616;
const NOLLIE1_OS2_1: u16 = 0x2A01;
const NOLLIE8_OS2_1: u16 = 0x2A08;
const PRISM8_OS2_1: u16 = 0x2C08;
const NOLLIE16_OS2_1: u16 = 0x2A16;
const NOLLIE32_OS2_1: u16 = 0x2A32;
const FLAG1: usize = 15;
const FLAG2: usize = 31;
const MOS_CHANNEL: usize = 26;
const MOS_LEDS: usize = 20;
/// OpenRGB starts every channel at 0 LEDs; this is the count assumed here, capped by the model.
const CHANNEL_LEDS: usize = 60;

const DFLT: [usize; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
const CH32: [usize; 32] = [
    5, 4, 3, 2, 1, 0, 15, 14, 26, 27, 28, 29, 30, 31, 8, 9, 19, 18, 17, 16, 7, 6, 25, 24, 23, 22,
    21, 20, 13, 12, 11, 10,
];
const CH16: [usize; 32] = [
    19, 18, 17, 16, 24, 25, 26, 27, 20, 21, 22, 23, 31, 30, 29, 28, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,
    10, 11, 12, 13, 14, 15,
];
const N16: [usize; 16] = [3, 2, 1, 0, 8, 9, 10, 11, 4, 5, 6, 7, 15, 14, 13, 12];

/// (vid, pid, channels, LEDs per channel, channel index) from `NollieDevices.cpp`.
const MODELS: [(u16, u16, usize, usize, &[usize]); 16] = [
    (HS_VID, NOLLIE32, 32, 256, &CH32),
    (HS_VID, NOLLIE16, 16, 256, &CH16),
    (FS_VID, NOLLIE8, 8, 126, &DFLT),
    (FS_VID, NOLLIE1, 1, 630, &DFLT),
    (FS_VID, NOLLIE28_12, 1, 42, &DFLT),
    (FS_VID, 0x1617, 8, 525, &DFLT),
    (FS_VID, 0x1618, 8, 525, &DFLT),
    (OS2_VID, NOLLIE32, 32, 256, &CH32),
    (OS2_VID, NOLLIE16, 16, 256, &N16),
    (OS2_VID, NOLLIE8, 8, 126, &DFLT),
    (OS2_VID, NOLLIE1, 1, 630, &DFLT),
    (OS2_VID, NOLLIE32_OS2_1, 32, 256, &CH32),
    (OS2_VID, NOLLIE16_OS2_1, 16, 256, &N16),
    (OS2_VID, NOLLIE8_OS2_1, 8, 126, &DFLT),
    (OS2_VID, PRISM8_OS2_1, 8, 126, &DFLT),
    (OS2_VID, NOLLIE1_OS2_1, 1, 630, &DFLT),
];

fn high_speed(pid: u16) -> bool {
    matches!(pid, NOLLIE32 | NOLLIE16 | NOLLIE32_OS2_1 | NOLLIE16_OS2_1)
}

/// `NollieController::SendPacket`: one 1025-byte report per channel, GRB.
fn hs_packet(channel: usize, colors: &[Color]) -> Vec<Op> {
    let mut b = vec![0u8; 1025];
    b[1] = channel as u8;
    b[3] = (colors.len() / 256) as u8;
    b[4] = (colors.len() % 256) as u8;
    for (i, c) in colors.iter().enumerate() {
        b[5 + i * 3..8 + i * 3].copy_from_slice(&[c.1, c.0, c.2]);
    }
    match channel {
        FLAG1 | FLAG2 => {
            b[2] = if channel == FLAG1 { 1 } else { 2 };
            vec![Op::Write(b), Op::Sleep(8)]
        }
        _ => vec![Op::Write(b)],
    }
}

/// `NollieController::SendPacketFS`: 21 LEDs per 65-byte report.
fn fs_packet(pid: u16, channel: usize, packet: usize, colors: &[Color]) -> Vec<u8> {
    let interval = match pid {
        NOLLIE28_12 => 2,
        NOLLIE8 | NOLLIE8_OS2_1 | PRISM8_OS2_1 => 6,
        NOLLIE1 | NOLLIE1_OS2_1 => 30,
        _ => 25,
    };
    let grb = matches!(
        pid,
        NOLLIE8 | NOLLIE1 | NOLLIE8_OS2_1 | NOLLIE1_OS2_1 | PRISM8_OS2_1
    );
    let mut b = vec![0u8; 65];
    b[1] = (packet + channel * interval) as u8;
    for (i, c) in colors.iter().enumerate() {
        let rgb = if grb {
            [c.1, c.0, c.2]
        } else {
            [c.0, c.1, c.2]
        };
        b[2 + i * 3..5 + i * 3].copy_from_slice(&rgb);
    }
    b
}

fn report(head: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

/// Each channel at the user's length, up to the model's maximum.
fn sized(id: &str, channels: usize, per_channel: usize) -> Vec<usize> {
    (0..channels)
        .map(|i| {
            let name = format!("Channel {}", i + 1);
            let default = CHANNEL_LEDS.min(per_channel);
            crate::lengths::length(id, i, &name, default, 0, per_channel)
        })
        .collect()
}

struct Nollie {
    vid: u16,
    pid: u16,
    zones: Vec<usize>,
    index: &'static [usize],
}

impl Nollie {
    fn zone(&self, colors: &[Color], z: usize) -> Vec<Color> {
        let from: usize = self.zones[..z].iter().sum();
        span(colors, from, self.zones[z])
    }
}

impl Protocol for Nollie {
    /// What `DeviceConfigureZone` sends once the channel sizes are set.
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        if self.vid == HS_VID && self.pid == NOLLIE32 {
            let on = self.zones[MOS_CHANNEL] <= MOS_LEDS;
            ops.push(Op::Write(report(&[0x80, u8::from(on)])));
        }
        if self.vid == FS_VID && self.pid == NOLLIE1 {
            let n = self.zones[0];
            ops.push(Op::Write(report(&[0xFE, 0x03, n as u8, (n >> 8) as u8])));
        }
        ops
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        if high_speed(self.pid) {
            let mut channels: Vec<usize> = (0..self.zones.len())
                .filter(|&z| {
                    let ch = self.index[z];
                    self.zones[z] > 0 || ch == FLAG1 || ch == FLAG2
                })
                .map(|z| self.index[z])
                .collect();
            channels.sort_unstable();
            for ch in channels {
                let z = self.index.iter().position(|&c| c == ch).unwrap_or(0);
                ops.extend(hs_packet(ch, &self.zone(colors, z)));
            }
        } else {
            for z in 0..self.zones.len() {
                let zone = self.zone(colors, z);
                for (p, chunk) in zone.chunks(21).enumerate() {
                    ops.push(Op::Write(fs_packet(self.pid, z, p, chunk)));
                }
            }
            ops.push(Op::Write(report(&[0xFF])));
        }
        ops
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let &(vid, pid, channels, per_channel, index) = MODELS
        .iter()
        .find(|m| m.0 == info.vid && m.1 == info.pid)
        .ok_or("unknown Nollie model")?;
    open_with(api, info, "nollie", |_| {
        let id = info.device_id("nollie");
        let zones = sized(&id, channels, per_channel);
        let (leds, grid) = rows(&zones);
        let dev = device(id, d.name, DeviceKind::Strip, grid, leds);
        Ok((
            dev,
            Nollie {
                vid,
                pid,
                zones,
                index,
            },
        ))
    })
}

pub(super) static DETECTORS: [Detector; 16] = [
    any("Nollie 32CH", HS_VID, NOLLIE32, open),
    any("Nollie 16CH", HS_VID, NOLLIE16, open),
    any("Nollie 8CH", FS_VID, NOLLIE8, open),
    any("Nollie 1CH", FS_VID, NOLLIE1, open),
    any("Nollie 28 12", FS_VID, NOLLIE28_12, open),
    any("Nollie 28 L1", FS_VID, 0x1617, open),
    any("Nollie 28 L2", FS_VID, 0x1618, open),
    any("Nollie 32_OS2", OS2_VID, NOLLIE32, open),
    any("Nollie 16_OS2", OS2_VID, NOLLIE16, open),
    any("Nollie 8_OS2", OS2_VID, NOLLIE8, open),
    any("Nollie 1_OS2", OS2_VID, NOLLIE1, open),
    on_i("Nollie 32_OS2_1", OS2_VID, NOLLIE32_OS2_1, 0, open),
    on_i("Nollie 16_OS2_1", OS2_VID, NOLLIE16_OS2_1, 0, open),
    on_i("Nollie 8_OS2_1", OS2_VID, NOLLIE8_OS2_1, 2, open),
    on_i("Prism8 8_OS2_1", OS2_VID, PRISM8_OS2_1, 2, open),
    on_i("Nollie 1_OS2_1", OS2_VID, NOLLIE1_OS2_1, 2, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    fn nollie(vid: u16, pid: u16) -> Nollie {
        let &(_, _, channels, per, index) =
            MODELS.iter().find(|m| m.0 == vid && m.1 == pid).unwrap();
        Nollie {
            vid,
            pid,
            zones: vec![CHANNEL_LEDS.min(per); channels],
            index,
        }
    }

    // DeviceConfigureZone: SetMos(false) once channel 26 holds more than 20 LEDs
    #[test]
    fn nollie32_setup_turns_mos_off() {
        assert_eq!(
            nollie(HS_VID, NOLLIE32).setup(&[]),
            vec![Op::Write(buf(65, &[0x00, 0x80, 0x00]))]
        );
        assert!(nollie(OS2_VID, NOLLIE32).setup(&[]).is_empty());
    }

    // DeviceConfigureZone: InitChLEDs on the Nollie 1
    #[test]
    fn nollie1_setup_reports_count() {
        let ops = nollie(FS_VID, NOLLIE1).setup(&[]);
        assert_eq!(ops, vec![Op::Write(buf(65, &[0x00, 0xFE, 0x03, 60, 0]))]);
    }

    // DeviceUpdateLEDs for the 32-channel model: channels sorted, flag channels marked
    #[test]
    fn hs_frame_order_and_flags() {
        let mut n = nollie(HS_VID, NOLLIE32);
        let colors: Vec<Color> = (0..32 * 60)
            .map(|i| Color((i / 60) as u8, i as u8, 9))
            .collect();
        let ops = n.frame(&colors);
        // 32 reports plus a sleep after channels 15 and 31
        assert_eq!(ops.len(), 34);
        let first = bytes(&ops[0]);
        assert_eq!(first.len(), 1025);
        // channel 0 is zone 5 in ch32
        assert_eq!(first[..5], [0x00, 0, 0, 0, 60]);
        assert_eq!(first[5..8], [5 * 60, 5, 9].map(|v: usize| v as u8));
        let flag1 = bytes(&ops[15]);
        assert_eq!(flag1[..3], [0x00, 15, 1]);
        // channel 15 is zone 6
        assert_eq!(flag1[5..8], [(6 * 60) as u8, 6, 9]);
        assert_eq!(ops[16], Op::Sleep(8));
        let flag2 = bytes(&ops[32]);
        assert_eq!(flag2[..3], [0x00, 31, 2]);
        assert_eq!(ops[33], Op::Sleep(8));
        assert!(flag2[5 + 60 * 3..].iter().all(|&b| b == 0));
    }

    // SetChannelLEDs -> SendPacketFS on the Nollie 8 (GRB, interval 6), then SendUpdate
    #[test]
    fn fs_frame_nollie8() {
        let mut n = nollie(FS_VID, NOLLIE8);
        let colors: Vec<Color> = (0..8 * 60).map(|i| Color(i as u8, 100, 200)).collect();
        let ops = n.frame(&colors);
        assert_eq!(ops.len(), 8 * 3 + 1);
        let p = bytes(&ops[4]);
        // zone 1, packet 1
        assert_eq!(p[..5], [0x00, 1 + 6, 100, 81, 200]);
        let last = bytes(&ops[23]);
        assert_eq!(last[1], 2 + 7 * 6);
        assert_eq!(last[2..5], [100, (7 * 60 + 42) as u8, 200]);
        assert!(last[2 + 18 * 3..].iter().all(|&b| b == 0));
        assert_eq!(bytes(&ops[24]), buf(65, &[0x00, 0xFF]).as_slice());
    }

    #[test]
    fn a_set_length_sizes_a_channel() {
        crate::lengths::set_one("hid:nollie-test", 0, 300);
        let zones = sized("hid:nollie-test", 1, 630);
        assert_eq!(zones, [300]);
        let mut n = Nollie {
            zones,
            ..nollie(FS_VID, NOLLIE1)
        };
        let ops = n.setup(&[]);
        assert_eq!(ops, vec![Op::Write(buf(65, &[0x00, 0xFE, 0x03, 44, 1]))]);
        assert_eq!(n.frame(&[]).len(), 300usize.div_ceil(21) + 1);
    }

    // SendPacketFS on the 28 L1 keeps RGB order with interval 25
    #[test]
    fn fs_packet_l1_is_rgb() {
        let p = fs_packet(0x1617, 3, 1, &[Color(1, 2, 3)]);
        assert_eq!(p, buf(65, &[0x00, 1 + 3 * 25, 1, 2, 3]));
    }
}
