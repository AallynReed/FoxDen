//! Ported from OpenRGB Controllers/DRGBController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, any, at, device, open_with, rows};
use crate::registry::{Detector, Info};

const V2_VID: u16 = 0x2023;
const V4_VID: u16 = 0x2486;
const YICO_VID: u16 = 0x1368;
/// OpenRGB starts every channel at 0 LEDs and keeps the count in a byte; this is the count assumed.
const CHANNEL_LEDS: usize = 60;
const V4_FIRST: usize = 316;
const V4_NEXT: usize = 340;
const FS_PER_PACKET: usize = 21;
/// LEDs that fit OpenRGB's RGBData buffers: 24 V4 packets, and 1801 colours for V1/V3.
const V4_MAX_TOTAL: usize = V4_FIRST + 23 * V4_NEXT;
const FS_MAX_TOTAL: usize = 1800;

/// (pid, channels, LEDs per channel, protocol version) from `DRGBDevices.cpp`.
const DEVICES: [(u16, usize, usize, u8); 33] = [
    (0x3608, 8, 512, 4),
    (0x3616, 16, 256, 4),
    (0x3628, 32, 256, 4),
    (0x3636, 36, 256, 4),
    (0x3204, 4, 256, 4),
    (0x3216, 16, 256, 4),
    (0x3F08, 8, 256, 4),
    (0x3F16, 8, 256, 4),
    (0x3F28, 16, 256, 4),
    (0x3208, 8, 512, 4),
    (0x3215, 16, 256, 4),
    (0x3217, 16, 256, 4),
    (0x3228, 32, 256, 4),
    (0x3229, 32, 256, 4),
    (0x3232, 32, 256, 4),
    (0x1209, 8, 256, 3),
    (0x1221, 16, 256, 3),
    (0x1226, 30, 256, 3),
    (0x1408, 8, 132, 1),
    (0x1407, 8, 132, 1),
    (0x1416, 16, 132, 1),
    (0x1410, 10, 132, 1),
    (0x1412, 12, 60, 1),
    (0x1208, 8, 256, 2),
    (0x1220, 16, 256, 2),
    (0x1210, 16, 256, 2),
    (0x1211, 6, 256, 2),
    (0x1215, 6, 256, 2),
    (0x6077, 8, 256, 3),
    (0x6078, 8, 256, 3),
    (0x6079, 8, 132, 3),
    (0x1614, 14, 132, 1),
    (0x1616, 16, 256, 4),
];

fn rgb(colors: &[Color], zones: &[usize]) -> Vec<u8> {
    let total: usize = zones.iter().sum();
    (0..total)
        .flat_map(|i| {
            let c = at(colors, i);
            [c.0, c.1, c.2]
        })
        .collect()
}

/// Version 4: `RGBData` (count header, then colours) cut into 1025-byte `SendPacket` reports.
fn v4(colors: &[Color], zones: &[usize]) -> Vec<Vec<u8>> {
    let mut data = vec![0u8; 8192 * 3 + 72];
    for (z, &n) in zones.iter().enumerate() {
        data[z * 2] = (n >> 8) as u8;
        data[z * 2 + 1] = n as u8;
    }
    let colours = rgb(colors, zones);
    data[72..72 + colours.len()].copy_from_slice(&colours);
    let total: usize = zones.iter().sum();
    let packets = if total > V4_FIRST {
        1 + (total - V4_FIRST).div_ceil(V4_NEXT)
    } else {
        1
    };
    let mut high = u8::from(total / 256 >= 1);
    let mut low = if total >= V4_FIRST {
        60
    } else {
        (total % 256) as u8
    };
    let mut rest = total.saturating_sub(V4_FIRST);
    (0..packets)
        .map(|i| {
            let mut b = vec![0u8; 1025];
            b[1] = (i + 100) as u8;
            b[2] = (packets + 99) as u8;
            b[3] = high;
            b[4] = low;
            b[5..].copy_from_slice(&data[i * 1020..i * 1020 + 1020]);
            if rest > 0 {
                high = u8::from(rest / 256 >= 1);
                low = if rest >= V4_NEXT {
                    84
                } else {
                    (rest % 256) as u8
                };
                rest = rest.saturating_sub(V4_NEXT);
            }
            b
        })
        .collect()
}

fn fs_report(payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1..1 + payload.len()].copy_from_slice(payload);
    b
}

/// `SendPacketFS` with an array id: 63-byte slices, the last one marked by `array + offset + i`.
fn fs_array(data: &[u8], packets: usize, array: u8) -> Vec<Vec<u8>> {
    let offset: usize = if array == 0x64 { 100 } else { 92 };
    let mut padded = data.to_vec();
    padded.resize(packets * 63, 0);
    (0..packets)
        .map(|i| {
            let id = if i == packets - 1 {
                array as usize + offset + i
            } else {
                array as usize + i
            };
            let mut b = vec![0u8; 65];
            b[1] = id as u8;
            b[2..].copy_from_slice(&padded[i * 63..i * 63 + 63]);
            b
        })
        .collect()
}

/// Versions 1 and 3: a count report (`46`/`60 BB ...`), then the colours under array `47`/`64`.
fn v1_v3(colors: &[Color], zones: &[usize], head: u8, array: u8) -> Vec<Vec<u8>> {
    let mut counts = vec![0u8; 64];
    counts[0] = head;
    counts[1] = 0xBB;
    for (z, &n) in zones.iter().enumerate() {
        counts[z * 2 + 2] = (n >> 8) as u8;
        counts[z * 2 + 3] = n as u8;
    }
    let data = rgb(colors, zones);
    let leds = data.len() / 3;
    let mut out = vec![fs_report(&counts)];
    if leds > 0 {
        out.extend(fs_array(&data, leds.div_ceil(FS_PER_PACKET), array));
    }
    out
}

/// Version 2: per channel, packets of 20 LEDs as `cur, total, channel, BB, colours`.
fn v2(colors: &[Color], zones: &[usize]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut from = 0;
    for (z, &n) in zones.iter().enumerate() {
        let mut data: Vec<u8> = (from..from + n)
            .flat_map(|i| {
                let c = at(colors, i);
                [c.0, c.1, c.2]
            })
            .collect();
        from += n;
        let packets = n.div_ceil(20);
        data.resize(packets * 60, 0);
        for p in 0..packets {
            let mut a = vec![p as u8 + 1, packets as u8, z as u8, 0xBB];
            a.extend_from_slice(&data[p * 60..p * 60 + 60]);
            out.push(fs_report(&a));
        }
    }
    out
}

/// The keepalive report: `0x65`, the rest of OpenRGB's uninitialised buffer sent as zeros.
fn sleep_report() -> Vec<u8> {
    fs_report(&[0x65])
}

/// Each channel at the user's length, within the byte count and the buffers above.
fn sized(id: &str, channels: usize, per_channel: usize, version: u8) -> Vec<usize> {
    let total = match version {
        4 => V4_MAX_TOTAL,
        2 => usize::MAX,
        _ => FS_MAX_TOTAL,
    };
    let max = per_channel.min(255).min(total / channels.max(1));
    (0..channels)
        .map(|i| {
            let name = format!("Channel {}", i + 1);
            crate::lengths::length(id, i, &name, CHANNEL_LEDS.min(max), 0, max)
        })
        .collect()
}

struct Drgb {
    version: u8,
    zones: Vec<usize>,
}

impl Protocol for Drgb {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let packets = match self.version {
            4 => v4(colors, &self.zones),
            3 => v1_v3(colors, &self.zones, 0x60, 0x64),
            2 => v2(colors, &self.zones),
            _ => v1_v3(colors, &self.zones, 0x46, 0x47),
        };
        packets.into_iter().map(Op::Write).collect()
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_millis(500))
    }

    fn refresh(&mut self, _: &[Color]) -> Vec<Op> {
        vec![Op::Write(sleep_report())]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let &(_, channels, per_channel, version) = DEVICES
        .iter()
        .find(|e| e.0 == info.pid)
        .ok_or("unknown DRGB model")?;
    open_with(api, info, "drgb", |_| {
        let id = info.device_id("drgb");
        let zones = sized(&id, channels, per_channel, version);
        let (leds, grid) = rows(&zones);
        let dev = device(id, d.name, DeviceKind::Strip, grid, leds);
        Ok((dev, Drgb { version, zones }))
    })
}

pub(super) static DETECTORS: [Detector; 33] = [
    any("DeepRGB LED V4", V4_VID, 0x3608, open),
    any("DeepRGB ULTRA V4F", V4_VID, 0x3616, open),
    any("DeepRGB CORE V4F", V4_VID, 0x3628, open),
    any("DeepRGB SIG V4F", V4_VID, 0x3636, open),
    any("Airgoo AG-DRGB04", V4_VID, 0x3204, open),
    any("Airgoo AG-DRGB16", V4_VID, 0x3216, open),
    any("Airgoo AG-DRGB08", V4_VID, 0x3F08, open),
    any("Airgoo AG-F8-DRGB08", V4_VID, 0x3F16, open),
    any("Airgoo AG-F12-DRGB16", V4_VID, 0x3F28, open),
    any("DeepRGB L8 V5", V4_VID, 0x3208, open),
    any("DeepRGB U16 V5", V4_VID, 0x3215, open),
    any("DeepRGB U16 V5F", V4_VID, 0x3217, open),
    any("DeepRGB C16 V5", V4_VID, 0x3228, open),
    any("DeepRGB C16 V5F", V4_VID, 0x3229, open),
    any("DeepRGB S16 V5F", V4_VID, 0x3232, open),
    any("DeepRGB LED", V2_VID, 0x1209, open),
    any("DeepRGB Ultra V3", V2_VID, 0x1221, open),
    any("DeepRGB CORE V3", V2_VID, 0x1226, open),
    any("DeepRGB E8 F", V2_VID, 0x1408, open),
    any("DeepRGB E8", V2_VID, 0x1407, open),
    any("DeepRGB E16", V2_VID, 0x1416, open),
    any("NEEDMAX 10 ELITE", V2_VID, 0x1410, open),
    any("JPU ELITE", V2_VID, 0x1412, open),
    any("DeepRGB LED Controller", V2_VID, 0x1208, open),
    any("DeepRGB ULTRA", V2_VID, 0x1220, open),
    any("DeepRGB SIG AB", V2_VID, 0x1210, open),
    any("DeepRGB SIG CD", V2_VID, 0x1211, open),
    any("DeepRGB Strimer Controller", V2_VID, 0x1215, open),
    any("YICO 8 ELITE", YICO_VID, 0x6077, open),
    any("YICO 08 ELITE", YICO_VID, 0x6078, open),
    any("YICO 08 ELITE", YICO_VID, 0x6079, open),
    any("YICO 14 LCD", V2_VID, 0x1614, open),
    any("YICO 16 ELITE", V4_VID, 0x1616, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    fn ramp(n: usize) -> Vec<Color> {
        (0..n).map(|i| Color(i as u8, (i >> 8) as u8, 7)).collect()
    }

    // RGBController_DRGB::DeviceUpdateLEDs case 4 + DRGBController::SendPacket, 2 x 200 LEDs
    #[test]
    fn v4_two_packets() {
        let colors = ramp(400);
        let ops = Drgb {
            version: 4,
            zones: vec![200, 200],
        }
        .frame(&colors);
        assert_eq!(ops.len(), 2);
        let a = bytes(&ops[0]);
        assert_eq!(a.len(), 1025);
        assert_eq!(a[..5], [0x00, 100, 101, 1, 60]);
        assert_eq!(a[5..9], [0, 200, 0, 200]);
        assert!(a[9..5 + 72].iter().all(|&b| b == 0));
        assert_eq!(a[77..80], [0, 0, 7]);
        // LED 315 is the last one in the first packet
        assert_eq!(a[1022..1025], [59, 1, 7]);
        let b = bytes(&ops[1]);
        // 84 LEDs remain
        assert_eq!(b[..5], [0x00, 101, 101, 0, 84]);
        assert_eq!(b[5..8], [60, 1, 7]);
        assert_eq!(b[5 + 83 * 3..5 + 84 * 3], [143, 1, 7]);
        assert!(b[5 + 84 * 3..].iter().all(|&x| x == 0));
    }

    // SendPacket with fewer than 256 LEDs
    #[test]
    fn v4_small() {
        let ops = Drgb {
            version: 4,
            zones: vec![10],
        }
        .frame(&ramp(10));
        assert_eq!(bytes(&ops[0])[..7], [0x00, 100, 100, 0, 10, 0, 10]);
    }

    // Case 3: SendPacketFS(ArrayData, 1, 0) then SendPacketFS(RGBData, n, 0x64)
    #[test]
    fn v3_packets() {
        let ops = Drgb {
            version: 3,
            zones: vec![30, 0, 2],
        }
        .frame(&ramp(32));
        assert_eq!(ops.len(), 3);
        assert_eq!(
            bytes(&ops[0]),
            buf(65, &[0x00, 0x60, 0xBB, 0, 30, 0, 0, 0, 2]).as_slice()
        );
        let a = bytes(&ops[1]);
        assert_eq!(a[..5], [0x00, 0x64, 0, 0, 7]);
        assert_eq!(a[62..65], [20, 0, 7]);
        let b = bytes(&ops[2]);
        assert_eq!(b[..5], [0x00, 0x64 + 100 + 1, 21, 0, 7]);
        assert_eq!(b[32..35], [31, 0, 7]);
        assert!(b[35..].iter().all(|&x| x == 0));
    }

    // Case 1: header 0x46, array 0x47 with offset 92
    #[test]
    fn v1_packets() {
        let ops = Drgb {
            version: 1,
            zones: vec![1],
        }
        .frame(&[Color(1, 2, 3)]);
        assert_eq!(
            bytes(&ops[0]),
            buf(65, &[0x00, 0x46, 0xBB, 0, 1]).as_slice()
        );
        assert_eq!(
            bytes(&ops[1]),
            buf(65, &[0x00, 0x47 + 92, 1, 2, 3]).as_slice()
        );
    }

    // Case 2: per channel, CurrPacket, NumPackets, zone, 0xBB, 60 colour bytes
    #[test]
    fn v2_packets() {
        let ops = Drgb {
            version: 2,
            zones: vec![0, 21],
        }
        .frame(&ramp(21));
        assert_eq!(ops.len(), 2);
        let a = bytes(&ops[0]);
        assert_eq!(a[..8], [0x00, 1, 2, 1, 0xBB, 0, 0, 7]);
        assert_eq!(a[62..65], [19, 0, 7]);
        assert_eq!(
            bytes(&ops[1]),
            buf(65, &[0x00, 2, 2, 1, 0xBB, 20, 0, 7]).as_slice()
        );
    }

    // KeepaliveThread: SendPacketFS(sleep_buf, 1, 0) after 500 ms
    #[test]
    fn keepalive_sleep_packet() {
        let mut d = Drgb {
            version: 4,
            zones: vec![],
        };
        assert_eq!(d.keepalive(), Some(Duration::from_millis(500)));
        assert_eq!(d.refresh(&[]), vec![Op::Write(buf(65, &[0x00, 0x65]))]);
    }

    #[test]
    fn a_set_length_sizes_a_channel_within_the_buffers() {
        crate::lengths::set_one("hid:drgb-test", 1, 5);
        crate::lengths::set_one("hid:drgb-test", 2, 900);
        assert_eq!(sized("hid:drgb-test", 3, 512, 2), [60, 5, 255]);
        crate::lengths::set_one("hid:drgb-test-v4", 0, 900);
        let zones = sized("hid:drgb-test-v4", 36, 256, 4);
        assert_eq!(zones[..2], [226, 60]);
        let mut full = Drgb {
            version: 4,
            zones: vec![226; 36],
        };
        assert_eq!(full.frame(&[]).len(), 24);
    }

    #[test]
    fn default_counts_fit_openrgb_buffers() {
        for &(_, channels, per, version) in &DEVICES {
            let n = CHANNEL_LEDS.min(per);
            assert!(n < 256);
            match version {
                4 => assert!(channels * n <= 8192 && channels <= 36),
                1 | 3 => assert!(channels * n <= 1800 && channels <= 31),
                _ => {}
            }
        }
    }
}
