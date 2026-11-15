//! Ported from OpenRGB Controllers/NZXTHue2Controller (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Op, Protocol, any, ask, device, open_with, rows, span};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1E71;
/// Zones top out at 40 LEDs; the C++ colour buffer overflows past that.
const MAX_LEDS: usize = 40;

/// LEDs of each accessory id in the device list.
fn accessory_leds(id: u8) -> usize {
    match id {
        0x01 | 0x04 | 0x0A => 10,
        0x02 | 0x05 | 0x0B | 0x0C | 0x10 | 0x17 | 0x18 | 0x19 => 8,
        0x06 => 6,
        0x08 => 14,
        0x09 => 15,
        0x11 => 1,
        0x13 | 0x14 => 18,
        0x15 | 0x16 => 20,
        0x1C => 16,
        0x1D..=0x1F => 24,
        _ => 0,
    }
}

/// `NZXTHue2Controller::UpdateDeviceList`: LEDs on each channel from the `21 03` reply.
fn channel_leds(reply: &[u8], channels: usize) -> Vec<usize> {
    (0..channels)
        .map(|ch| {
            let start = 0x0F + 6 * ch;
            let n: usize = (0..6)
                .map(|d| accessory_leds(reply.get(start + d).copied().unwrap_or(0)))
                .sum();
            n.min(MAX_LEDS)
        })
        .collect()
}

fn request(head: [u8; 2]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..2].copy_from_slice(&head);
    b
}

/// `NZXTHue2Controller::SendDirect`.
fn direct(channel: usize, group: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x22;
    b[1] = 0x10 | group;
    b[2] = 1 << channel;
    for (i, c) in colors.iter().enumerate() {
        b[4 + i * 3..7 + i * 3].copy_from_slice(&[c.1, c.0, c.2]);
    }
    b
}

/// `NZXTHue2Controller::SendApply`.
fn apply(channel: usize) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x22;
    b[1] = 0xA0;
    b[2] = 1 << channel;
    b[0x04] = 0x01;
    b[0x07] = 0x28;
    b[0x0A] = 0x80;
    b[0x0C] = 0x32;
    b[0x0F] = 0x01;
    b
}

/// The counts the accessories gave, or the user's.
fn sized(id: &str, found: Vec<usize>) -> Vec<usize> {
    found
        .into_iter()
        .enumerate()
        .map(|(i, n)| {
            let name = format!("Channel {}", i + 1);
            crate::lengths::length(id, i, &name, n, 0, MAX_LEDS)
        })
        .collect()
}

struct Hue2 {
    leds: Vec<usize>,
}

impl Protocol for Hue2 {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        let mut from = 0;
        for (ch, &n) in self.leds.iter().enumerate() {
            let zone = span(colors, from, n);
            from += n;
            ops.push(Op::Write(direct(ch, 0, &zone[..n.min(20)])));
            if n > 20 {
                ops.push(Op::Write(direct(ch, 1, &zone[20..])));
            }
            ops.push(Op::Write(apply(ch)));
        }
        ops
    }
}

/// `SendFirmwareRequest` and `UpdateDeviceList`, with the replies each waits for.
fn handshake_packets() -> [(Vec<u8>, [u8; 2]); 2] {
    [
        (request([0x10, 0x01]), [0x11, 0x01]),
        (request([0x20, 0x03]), [0x21, 0x03]),
    ]
}

fn handshake(hid: &HidDevice, channels: usize) -> Result<Vec<usize>, String> {
    let [(firmware, fw_reply), (list, list_reply)] = handshake_packets();
    ask(hid, &firmware, 64, 1000, 10, |r| {
        r.len() == 64 && r[..2] == fw_reply
    })?;
    let reply = ask(hid, &list, 64, 1000, 10, |r| {
        r.len() == 64 && r[..2] == list_reply
    })
    .unwrap_or_default();
    Ok(channel_leds(&reply, channels))
}

fn open(
    api: &HidApi,
    info: &Info,
    d: &Detector,
    channels: usize,
) -> Result<Box<dyn Backend>, String> {
    open_board(api, info, d, channels, 0)
}

/// `argb` plain ARGB headers follow the accessory channels; they report no accessories, so get 40 LEDs.
fn open_board(
    api: &HidApi,
    info: &Info,
    d: &Detector,
    channels: usize,
    argb: usize,
) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "nzxt hue 2", |hid| {
        let mut found = handshake(hid, channels)?;
        found.extend(std::iter::repeat_n(MAX_LEDS, argb));
        if found.iter().sum::<usize>() == 0 {
            return Err("no accessories".into());
        }
        let id = info.device_id("nzxt-hue2");
        let leds = sized(&id, found);
        let (pos, grid) = rows(&leds);
        let dev = device(id, d.name, DeviceKind::Strip, grid, pos);
        Ok((dev, Hue2 { leds }))
    })
}

fn open2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, 2)
}

fn open3(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, 3)
}

fn open4(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, 4)
}

fn open6(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, 6)
}

/// NZXT_Motherboard.js ChannelLibrary: NZXT accessory headers, then ARGB headers.
fn open_n2a1(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_board(api, info, d, 2, 1)
}

fn open_n4a2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_board(api, info, d, 4, 2)
}

fn open_n1a2(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_board(api, info, d, 1, 2)
}

pub(super) static DETECTORS: [Detector; 27] = [
    any("NZXT RGB & Fan Controller", VID, 0x2009, open2),
    any("NZXT RGB & Fan Controller", VID, 0x2010, open2),
    any("NZXT RGB & Fan Controller", VID, 0x200E, open2),
    any("NZXT RGB & Fan Controller", VID, 0x2011, open6),
    any("NZXT RGB & Fan Controller", VID, 0x2019, open6),
    any("NZXT RGB & Fan Controller", VID, 0x2020, open6),
    any("NZXT RGB & Fan Controller", VID, 0x201F, open6),
    any("NZXT RGB & Fan Controller 2024", VID, 0x2022, open6),
    any("NZXT B650E Motherboard", VID, 0x201B, open6),
    any("NZXT Hue 2", VID, 0x2001, open4),
    any("NZXT Hue 2 Ambient", VID, 0x2002, open2),
    any("NZXT Hue 2 Motherboard", VID, 0x2005, open2),
    any("NZXT Hue 2 Motherboard", VID, 0x200B, open2),
    any("NZXT N7 Z490 Motherboard", VID, 0x200A, open_n2a1),
    any("NZXT N7 Z590 Motherboard", VID, 0x200C, open_n2a1),
    any("NZXT N7 Z690 Motherboard", VID, 0x2016, open_n2a1),
    any("NZXT N5 Z690 Motherboard", VID, 0x2017, open_n2a1),
    any("NZXT N7 Z790 Motherboard", VID, 0x201D, open_n4a2),
    any("NZXT N7 B850 Motherboard", VID, 0x2026, open_n1a2),
    any("NZXT Kraken X3 Series", VID, 0x2007, open3),
    any("NZXT Kraken X3 Series RGB", VID, 0x2014, open3),
    any("NZXT Kraken 2024 ELITE Series RGB", VID, 0x3012, open2),
    any("NZXT RGB Controller", VID, 0x2012, open3),
    any("NZXT RGB Controller", VID, 0x2021, open3),
    any("NZXT Smart Device V2", VID, 0x2006, open2),
    any("NZXT Smart Device V2", VID, 0x200D, open2),
    any("NZXT Smart Device V2", VID, 0x200F, open2),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    // NZXTHue2Controller::SendFirmwareRequest and UpdateDeviceList requests
    #[test]
    fn handshake_requests() {
        let [(fw, fw_reply), (list, list_reply)] = handshake_packets();
        assert_eq!(fw, buf(64, &[0x10, 0x01]));
        assert_eq!(fw_reply, [0x11, 0x01]);
        assert_eq!(list, buf(64, &[0x20, 0x03]));
        assert_eq!(list_reply, [0x21, 0x03]);
    }

    // NZXTHue2Controller::UpdateDeviceList
    #[test]
    fn device_list_counts_leds() {
        let mut reply = buf(64, &[0x21, 0x03]);
        reply[0x0F] = 0x13; // F120 fan, 18
        reply[0x10] = 0x13;
        reply[0x0F + 6] = 0x04; // Hue 2 strip, 10
        reply[0x0F + 7] = 0x08; // cable comb, 14
        reply[0x0F + 8] = 0x7F; // unknown, 0
        reply[0x0F + 12] = 0x11; // X3 logo, 1
        assert_eq!(channel_leds(&reply, 4), vec![36, 24, 1, 0]);
        reply[0x0F + 18] = 0x1E;
        reply[0x0F + 19] = 0x1E;
        assert_eq!(channel_leds(&reply, 4)[3], 40);
    }

    #[test]
    fn a_set_length_overrides_the_found_one() {
        crate::lengths::set_one("hid:nzxt-hue2-test", 1, 99);
        crate::lengths::set_one("hid:nzxt-hue2-test", 2, 4);
        assert_eq!(sized("hid:nzxt-hue2-test", vec![36, 24, 0]), [36, 40, 4]);
    }

    // NZXTHue2Controller::SetChannelLEDs: SendDirect group 0/1, then SendApply
    #[test]
    fn frame_splits_at_twenty_and_applies() {
        let colors: Vec<Color> = (0..26).map(|i| Color(i, i + 1, i + 2)).collect();
        let ops = Hue2 { leds: vec![0, 26] }.frame(&colors);
        assert_eq!(ops.len(), 5);
        assert_eq!(
            bytes(&ops[0]),
            buf(64, &[0x22, 0x10, 0x01, 0x00]).as_slice()
        );
        let mut apply0 = buf(64, &[0x22, 0xA0, 0x01, 0x00, 0x01]);
        apply0[0x07] = 0x28;
        apply0[0x0A] = 0x80;
        apply0[0x0C] = 0x32;
        apply0[0x0F] = 0x01;
        assert_eq!(bytes(&ops[1]), apply0.as_slice());
        let g0 = bytes(&ops[2]);
        assert_eq!(g0[..4], [0x22, 0x10, 0x02, 0x00]);
        assert_eq!(g0[4..7], [1, 0, 2]);
        assert_eq!(g0[61..64], [20, 19, 21]);
        let g1 = bytes(&ops[3]);
        assert_eq!(g1[..4], [0x22, 0x11, 0x02, 0x00]);
        assert_eq!(g1[4..7], [21, 20, 22]);
        assert_eq!(g1[19..22], [26, 25, 27]);
        assert!(g1[22..].iter().all(|&b| b == 0));
        assert_eq!(bytes(&ops[4])[..3], [0x22, 0xA0, 0x02]);
    }
}
