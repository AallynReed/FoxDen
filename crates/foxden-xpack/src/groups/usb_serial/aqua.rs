//! Aqua Computer lighting over raw USB.

use super::link::open_bulk;
use crate::hid::{Driver, Io, Proto, set_rows, split};
use foxden_core::{Backend, Color, Device, DeviceKind};

const FAMILY: &str = "aqua-computer";
pub const VID: u16 = 0x0C70;
const ENDPOINT: u8 = 0x02;
const SET_LEDS: u8 = 0x0B;
const FIRST_SLOT: usize = 5;
const SLOT: usize = 4;

pub struct Model {
    pub name: &'static str,
    pub pid: u16,
    /// (channel byte, LED count) in send order.
    pub channels: &'static [(u8, usize)],
    pub packet_len: usize,
}

pub const MODELS: &[Model] = &[
    Model {
        name: "Aqua Computer D5 Next",
        pid: 0xF00E,
        channels: &[(0x00, ARGB_LEDS), (0x06, 6)],
        packet_len: 364,
    },
    Model {
        name: "Aqua Computer Farbwerk 360",
        pid: 0xF010,
        channels: &[
            (0, ARGB_LEDS),
            (1, ARGB_LEDS),
            (2, ARGB_LEDS),
            (3, ARGB_LEDS),
        ],
        packet_len: 365,
    },
    Model {
        name: "Aqua Computer Farbwerk Nano",
        pid: 0xF00F,
        channels: &[(0, ARGB_LEDS)],
        packet_len: 365,
    },
    Model {
        name: "Aqua Computer High Flow Next",
        pid: 0xF012,
        channels: &[(0x00, ARGB_LEDS), (0x06, 10)],
        packet_len: 364,
    },
];

/// R,B,G stream; slot k at 5+4k copies stream bytes from max(3k-1, 2), as the plugin does.
pub fn channel_packet(channel: u8, colors: &[Color], len: usize) -> Vec<u8> {
    let stream: Vec<u8> = colors.iter().flat_map(|c| [c.0, c.2, c.1]).collect();
    let mut p = vec![0u8; len];
    p[0] = SET_LEDS;
    p[1] = channel;
    p[2] = colors.len() as u8;
    for k in 0..colors.len() {
        let src = (3 * k).saturating_sub(1).max(2);
        for j in 0..3 {
            let at = FIRST_SLOT + SLOT * k + j;
            if at < len {
                p[at] = stream.get(src + j).copied().unwrap_or(0);
            }
        }
    }
    p
}

/// ARGB channels default to 90 LEDs, all the 364/365-byte packet holds (last slot at 5+4*89).
const ARGB_LEDS: usize = 90;

/// Each channel's count: the ARGB ones as the user set them, the built-in ones fixed.
pub fn counts(id: &str, m: &Model) -> Vec<usize> {
    let mut argb = 0;
    m.channels
        .iter()
        .enumerate()
        .map(|(i, &(_, n))| {
            if n != ARGB_LEDS {
                return n;
            }
            argb += 1;
            let name = format!("Channel {argb}");
            foxden_hid::zone_length(id, i, &name, ARGB_LEDS, 0, ARGB_LEDS)
        })
        .collect()
}

struct Aqua(&'static Model, Vec<usize>);

impl Aqua {
    fn packets(&self, colors: &[Color]) -> Vec<Vec<u8>> {
        split(colors, &self.1)
            .iter()
            .zip(self.0.channels)
            .map(|(zone, &(ch, _))| channel_packet(ch, zone, self.0.packet_len))
            .collect()
    }
    fn send(&self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in self.packets(colors) {
            io.write(&p, self.0.packet_len)?;
            io.sleep(1);
        }
        Ok(())
    }
}

impl Proto for Aqua {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.send(io, colors)
    }
    // the plugin streams every frame
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
    // Shutdown sends the default shutdown colour, black
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.send(io, &[]);
    }
}

pub fn device(id: &str, m: &Model) -> Device {
    let rows = m.channels.len();
    let mut dev = Device::grid(
        id,
        m.name,
        DeviceKind::Strip,
        [0.06, 0.40, 0.60, 0.04 * rows as f32],
        1,
        1,
    );
    set_rows(&mut dev, &counts(id, m));
    dev
}

/// The D5 Next pump has fans too, so the fan hubs' group opens its lighting.
pub const WITH_FANS: u16 = 0xF00E;

/// Every model `want` accepts.
pub fn probe(want: impl Fn(u16) -> bool) -> Vec<Box<dyn Backend>> {
    let mut out = Vec::new();
    for m in MODELS.iter().filter(|m| want(m.pid)) {
        for f in open_bulk(FAMILY, VID, m.pid, ENDPOINT) {
            let dev = device(&f.id, m);
            let aqua = Aqua(m, counts(&f.id, m));
            out.push(Driver::new(Box::new(f.io), dev, FAMILY, aqua).boxed());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_layout() {
        let c = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let p = channel_packet(6, &c, 364);
        assert_eq!(p.len(), 364);
        assert_eq!(&p[..5], &[0x0B, 6, 3, 0, 0]);
        // stream R,B,G = 1,3,2, 4,6,5, 7,9,8
        assert_eq!(&p[5..9], &[2, 4, 6, 0]);
        assert_eq!(&p[9..13], &[2, 4, 6, 0]);
        assert_eq!(&p[13..17], &[5, 7, 9, 0]);
        assert!(p[17..].iter().all(|&b| b == 0));
    }

    #[test]
    fn full_channel_fits() {
        let p = channel_packet(0, &[Color(9, 9, 9); 90], 364);
        assert_eq!(p[2], 90);
        assert_eq!(p[5 + 4 * 89], 9);
        assert_eq!(channel_packet(0, &[Color(9, 9, 9); 90], 365).len(), 365);
    }

    #[test]
    fn short_channels_send_their_count() {
        let aqua = Aqua(&MODELS[0], vec![30, 6]);
        let c: Vec<Color> = (0..36).map(|i| Color(i, 0, 0)).collect();
        let p = aqua.packets(&c);
        assert_eq!(p.len(), 2);
        assert_eq!(&p[0][..3], &[0x0B, 0, 30]);
        assert_eq!(p[0], channel_packet(0, &c[..30], 364));
        assert_eq!(&p[1][..3], &[0x0B, 6, 6]);
        assert_eq!(p[1], channel_packet(6, &c[30..], 364));
        let full = Aqua(&MODELS[1], vec![ARGB_LEDS; 4]).packets(&[]);
        assert!(full.iter().all(|p| p[2] == 90 && p.len() == 365));
    }

    #[test]
    fn layouts() {
        for m in MODELS {
            let n: usize = m.channels.iter().map(|c| c.1).sum();
            assert_eq!(device("x", m).leds.len(), n);
        }
    }
}
