//! Ported from OpenRGB Controllers/CorsairLightingNodeController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, any, device, open_with, rows, span};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1B1C;
const CHANNELS: usize = 2;
/// OpenRGB starts each channel at 0 LEDs (max 204); this is the count assumed here.
const LEDS: usize = 60;
const MAX_LEDS: usize = 204;
const READ: Op = Op::Read(17, 15);

fn packet(head: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

/// `SendDirect`: one colour component of up to 50 LEDs.
fn direct(channel: u8, start: u8, component: u8, values: &[u8]) -> Vec<u8> {
    let mut b = packet(&[0x32, channel, start, values.len() as u8, component]);
    b[6..6 + values.len()].copy_from_slice(values);
    b
}

/// `SetChannelLEDs`: software port state, red/green/blue blocks of 50, then a commit.
fn channel(ch: u8, colors: &[Color]) -> Vec<Op> {
    let mut ops = vec![Op::Write(packet(&[0x38, ch, 0x02])), READ];
    for (block, chunk) in colors.chunks(50).enumerate() {
        let start = (block * 50) as u8;
        let parts: [Vec<u8>; 3] = [
            chunk.iter().map(|c| c.0).collect(),
            chunk.iter().map(|c| c.1).collect(),
            chunk.iter().map(|c| c.2).collect(),
        ];
        for (component, values) in parts.iter().enumerate() {
            ops.push(Op::Write(direct(ch, start, component as u8, values)));
            ops.push(READ);
        }
    }
    ops.push(Op::Write(commit()));
    ops.push(READ);
    ops
}

fn commit() -> Vec<u8> {
    packet(&[0x33, 0xFF])
}

/// Both channels at the user's length.
fn sized(id: &str) -> [usize; CHANNELS] {
    std::array::from_fn(|i| {
        let name = format!("Channel {}", i + 1);
        crate::lengths::length(id, i, &name, LEDS, 0, MAX_LEDS)
    })
}

struct Node {
    counts: [usize; CHANNELS],
}

impl Protocol for Node {
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        vec![Op::Write(packet(&[0x02])), READ]
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut from = 0;
        let mut ops = Vec::new();
        for (ch, &n) in self.counts.iter().enumerate() {
            ops.extend(channel(ch as u8, &span(colors, from, n)));
            from += n;
        }
        ops
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_secs(5))
    }

    fn refresh(&mut self, _: &[Color]) -> Vec<Op> {
        vec![Op::Write(commit()), READ]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "corsair lighting node", |_| {
        let id = info.device_id("corsair-lighting-node");
        let counts = sized(&id);
        let (leds, grid) = rows(&counts);
        let dev = device(id, d.name, DeviceKind::Strip, grid, leds);
        Ok((dev, Node { counts }))
    })
}

pub(super) static DETECTORS: [Detector; 7] = [
    any("Corsair Lighting Node Core", VID, 0x0C1A, open),
    any("Corsair Lighting Node Pro", VID, 0x0C0B, open),
    any("Corsair Commander Pro", VID, 0x0C10, open),
    any("Corsair LS100 Lighting Kit", VID, 0x0C1E, open),
    any("Corsair 1000D Obsidian", VID, 0x1D00, open),
    any("Corsair SPEC OMEGA RGB", VID, 0x1D04, open),
    any("Corsair LT100", VID, 0x0C23, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    // SendFirmwareRequest via WriteAndRead
    #[test]
    fn setup_requests_firmware() {
        assert_eq!(
            Node { counts: [LEDS; 2] }.setup(&[]),
            vec![Op::Write(buf(65, &[0x00, 0x02])), Op::Read(17, 15)]
        );
    }

    // SetChannelLEDs: port state, three SendDirect per 50 LEDs, SendCommit
    #[test]
    fn channel_matches_set_channel_leds() {
        let colors: Vec<Color> = (0..60).map(|i| Color(i, 100 + i, 200)).collect();
        let ops = channel(1, &colors);
        assert_eq!(ops.len(), 2 + 6 * 2 + 2);
        assert!(
            ops.iter()
                .skip(1)
                .step_by(2)
                .all(|o| *o == Op::Read(17, 15))
        );
        assert_eq!(
            bytes(&ops[0]),
            buf(65, &[0x00, 0x38, 0x01, 0x02]).as_slice()
        );
        let red = bytes(&ops[2]);
        assert_eq!(red[..6], [0x00, 0x32, 0x01, 0, 50, 0]);
        assert_eq!(red[6..56], (0..50).collect::<Vec<u8>>()[..]);
        assert!(red[56..].iter().all(|&b| b == 0));
        let green = bytes(&ops[4]);
        assert_eq!(green[..7], [0x00, 0x32, 0x01, 0, 50, 1, 100]);
        let blue2 = bytes(&ops[12]);
        assert_eq!(blue2[..6], [0x00, 0x32, 0x01, 50, 10, 2]);
        assert_eq!(blue2[6..16], [200; 10]);
        assert!(blue2[16..].iter().all(|&b| b == 0));
        assert_eq!(bytes(&ops[14]), buf(65, &[0x00, 0x33, 0xFF]).as_slice());
    }

    // An empty channel still gets port state and commit
    #[test]
    fn empty_channel() {
        let ops = channel(0, &[]);
        assert_eq!(
            ops,
            vec![
                Op::Write(buf(65, &[0x00, 0x38, 0x00, 0x02])),
                Op::Read(17, 15),
                Op::Write(buf(65, &[0x00, 0x33, 0xFF])),
                Op::Read(17, 15),
            ]
        );
    }

    // KeepaliveThread: SendCommit after 5 s without one
    #[test]
    fn keepalive_commits() {
        let mut n = Node { counts: [LEDS; 2] };
        assert_eq!(n.keepalive(), Some(Duration::from_secs(5)));
        assert_eq!(
            n.refresh(&[]),
            vec![Op::Write(buf(65, &[0x00, 0x33, 0xFF])), Op::Read(17, 15)]
        );
        assert_eq!(n.frame(&vec![Color::BLACK; 120]).len(), 2 * 16);
    }

    #[test]
    fn a_set_length_sizes_a_channel() {
        crate::lengths::set_one("hid:lighting-node-test", 0, 10);
        crate::lengths::set_one("hid:lighting-node-test", 1, 300);
        let counts = sized("hid:lighting-node-test");
        assert_eq!(counts, [10, 204]);
        let mut colors = vec![Color::BLACK; 214];
        colors[10] = Color(7, 0, 0);
        let ops = Node { counts }.frame(&colors);
        // channel 0: state, 3 parts, commit; channel 1: state, 5 blocks of 3, commit
        assert_eq!(ops.len(), 2 * (5 + 17));
        assert_eq!(bytes(&ops[2])[..7], [0x00, 0x32, 0x00, 0, 10, 0, 0]);
        assert_eq!(bytes(&ops[12])[..7], [0x00, 0x32, 0x01, 0, 50, 0, 7]);
    }
}
