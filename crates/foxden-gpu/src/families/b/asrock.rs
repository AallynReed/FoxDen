//! ASRock Radeon graphics cards: a controller at 0x36 on the card's I2C bus, driven with raw
//! 12-byte packets of command, zero, subcommand and payload.

use std::time::Duration;

use foxden_core::{Backend, Color};
use foxden_smbus::i2c::SharedBus;

use super::common::{
    AMD_GPU_VEN, Card, Driver, Gpu, Io, c, changed, detect_each, device_id, info,
    is_amd_gpu_i2c_bus, row, sleep,
};

#[rustfmt::skip]
pub const ASROCK: &[Card] = &[
    c("ASRock Radeon RX 6800 XT Phantom Gaming", 0x1002, 0x73BF, 0x1849, 0x5202, 0x36),
    c("ASRock Radeon RX 9070 XT Steel Legend", 0x1002, 0x7550, 0x1849, 0x5403, 0x36),
];

/// Cards whose firmware does not answer the channel query, with the channels that light.
const FIXED_CHANNELS: &[(u16, &[u8])] = &[(0x5202, &[6])];

const PACKET: usize = 12;
const REPLY: usize = 32;
const CMD_COLOR: u8 = 0x10;
const CMD_QUERY: u8 = 0x14;
const QUERY_CHANNELS: u8 = 0x01;
const MODE_STATIC: u8 = 0x01;
const BRIGHTNESS: u8 = 0x80;
/// Effect speed, effect direction and a fixed trailing byte.
const COLOR_TAIL: [u8; 3] = [0xFF, 0x00, 0x1A];

fn packet(cmd: u8, sub: u8, payload: &[u8]) -> [u8; PACKET] {
    let mut p = [0; PACKET];
    p[0] = cmd;
    p[2] = sub;
    p[3..3 + payload.len()].copy_from_slice(payload);
    p
}

fn color_packet(channel: u8, c: Color) -> [u8; PACKET] {
    let mut payload = vec![MODE_STATIC, c.0, c.1, c.2, BRIGHTNESS];
    payload.extend(COLOR_TAIL);
    packet(CMD_COLOR, channel, &payload)
}

/// The channel bitmap the controller reports in reply bytes 4 and 5.
fn query_channels(io: &Io, addr: u8) -> Vec<u8> {
    if !io.raw_write(addr, &packet(CMD_QUERY, QUERY_CHANNELS, &[])) {
        return Vec::new();
    }
    sleep(Duration::from_millis(20));
    let Some(reply) = io.raw_read(addr, REPLY) else {
        return Vec::new();
    };
    let bits = u16::from_le_bytes([reply[4], reply[5]]);
    (0..16).filter(|i| bits & (1 << i) != 0).collect()
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, ASROCK, |bus, card| {
        let i = info(bus);
        if i.pci_vendor != AMD_GPU_VEN || !is_amd_gpu_i2c_bus(&i) {
            return None;
        }
        // A plain read must answer before anything is written.
        let io = Io::new(bus);
        io.raw_read(card.addr, 1)?;
        let channels = match FIXED_CHANNELS.iter().find(|f| f.0 == card.subdev) {
            Some(&(_, fixed)) => fixed.to_vec(),
            None => query_channels(&io, card.addr),
        };
        if channels.is_empty() {
            return None;
        }
        let dev = row(
            device_id("asrock", bus, card.addr),
            card.name,
            channels.len() as u32,
        );
        Some(Gpu::boxed(
            bus,
            dev,
            Asrock {
                addr: card.addr,
                channels,
            },
        ))
    })
}

struct Asrock {
    addr: u8,
    channels: Vec<u8>,
}

impl Driver for Asrock {
    fn setup(&mut self, _: &Io, _: &[Color]) {}

    fn update(&mut self, io: &Io, colors: &[Color], prev: Option<&[Color]>) {
        for i in changed(colors, prev) {
            io.raw_write(self.addr, &color_packet(self.channels[i], colors[i]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const PG_6800: &str = "ASRock Radeon RX 6800 XT Phantom Gaming";
    const SL_9070: &str = "ASRock Radeon RX 9070 XT Steel Legend";

    fn reply(bits: u16) -> Vec<u8> {
        let mut r = vec![0x02, 0x00, 0x06, 0x00];
        r.extend(bits.to_le_bytes());
        r.resize(REPLY, 0);
        r
    }

    #[test]
    fn needs_amd_bus_and_an_answer() {
        let bus = Fake::new("Other", ids(ASROCK, PG_6800))
            .raw(0x36, &[&[0]])
            .shared();
        assert!(run(&[detect], &bus).is_empty());
        assert!(take().is_empty());
        let bus = Fake::new("AMD ADL", ids(ASROCK, SL_9070)).shared();
        assert!(run(&[detect], &bus).is_empty());
        assert_eq!(take(), s(&["rraw 36 1"]));
        let bus = Fake::new("AMD ADL", [0x1002, 0x7550, 0x1849, 0x5404])
            .raw(0x36, &[&[0]])
            .shared();
        assert!(run(&[detect], &bus).is_empty());
        assert!(take().is_empty());
    }

    // The 6800 XT ignores the query, so its channel is fixed and detection only reads.
    #[test]
    fn fixed_channel_frame() {
        let bus = Fake::new("AMD ADL", ids(ASROCK, PG_6800))
            .raw(0x36, &[&[0x02]])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(take(), s(&["rraw 36 1"]));
        assert_eq!(b.devices()[0].leds.len(), 1);
        assert_eq!(
            push(&mut b, &[Color(0x11, 0x22, 0x33)]),
            s(&["wraw 36 10 00 06 01 11 22 33 80 ff 00 1a 00"])
        );
        assert!(push(&mut b, &[Color(0x11, 0x22, 0x33)]).is_empty());
    }

    #[test]
    fn queried_channels_frame() {
        let bus = Fake::new("AMD ADL", ids(ASROCK, SL_9070))
            .raw(0x36, &[&[0x02], reply(0b1100_1000).as_slice()])
            .shared();
        let mut b = run(&[detect], &bus).remove(0);
        assert_eq!(
            take(),
            s(&[
                "rraw 36 1",
                "wraw 36 14 00 01 00 00 00 00 00 00 00 00 00",
                "sleep 20ms",
                "rraw 36 32",
            ])
        );
        assert_eq!(b.devices()[0].leds.len(), 3);
        let red = Color(0xFF, 0, 0);
        assert_eq!(
            push(&mut b, &[red, Color::BLACK, red]),
            s(&[
                "wraw 36 10 00 03 01 ff 00 00 80 ff 00 1a 00",
                "wraw 36 10 00 06 01 00 00 00 80 ff 00 1a 00",
                "wraw 36 10 00 07 01 ff 00 00 80 ff 00 1a 00",
            ])
        );
        assert_eq!(
            push(&mut b, &[red, red, red]),
            s(&["wraw 36 10 00 06 01 ff 00 00 80 ff 00 1a 00"])
        );
    }

    #[test]
    fn empty_bitmap_is_not_a_device() {
        let bus = Fake::new("AMD ADL", ids(ASROCK, SL_9070))
            .raw(0x36, &[&[0x02], reply(0).as_slice()])
            .shared();
        assert!(run(&[detect], &bus).is_empty());
    }
}
