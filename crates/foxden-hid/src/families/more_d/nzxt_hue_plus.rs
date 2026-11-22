// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::serial::{Port, Serial};
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "nzxt-hue-plus";
pub const IDS: (u16, u16) = (0x04D8, 0x00DF);
const BAUD: u32 = 256_000;
const PACKET: usize = 125;
const MODE_DIRECT: u8 = 0x0E;
const MAX_LEDS: usize = 40;

/// GetLEDsOnChannel: fans count 8 LEDs each, strips 10; a silent channel gets the 40 a header takes.
pub fn leds_on_channel(io: &mut dyn Io, channel: u8) -> usize {
    io.flush(true, false);
    if io.write(&[0x8D, channel]).is_err() {
        return MAX_LEDS;
    }
    io.sleep(50);
    let mut b = [0u8; 5];
    match io.read(&mut b, 0) {
        Ok(5) if b[3] == 0x01 => (b[4] as usize * 8).min(MAX_LEDS),
        Ok(5) => (b[4] as usize * 10).min(MAX_LEDS),
        _ => MAX_LEDS,
    }
}

/// SetChannelLEDs: one Direct packet, colours in GRB order.
pub fn channel_packet(channel: u8, colors: &[Color]) -> [u8; PACKET] {
    let mut b = [0u8; PACKET];
    b[..5].copy_from_slice(&[0x4B, channel + 1, MODE_DIRECT, 0, 0]);
    for (i, c) in colors.iter().take(MAX_LEDS).enumerate() {
        b[5 + i * 3..8 + i * 3].copy_from_slice(&[c.1, c.0, c.2]);
    }
    b
}

pub struct HuePlus {
    pub channels: [usize; 2],
}

impl Proto for HuePlus {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for (channel, &n) in self.channels.iter().enumerate() {
            io.write(&channel_packet(channel as u8, &colors[at..at + n]))?;
            io.sleep(5);
            at += n;
        }
        Ok(())
    }
}

/// The counts the controller reported, or the user's.
fn sized(id: &str, found: [usize; 2]) -> [usize; 2] {
    let mut i = 0;
    found.map(|n| {
        i += 1;
        crate::lengths::length(id, i - 1, &format!("Channel {i}"), n, 0, MAX_LEDS)
    })
}

pub fn open(port: &Port) -> Option<Box<dyn Backend>> {
    let mut io = Serial::open(&port.name, BAUD, false).ok()?;
    let id = named_id(FAMILY, &port.name);
    let found = [leds_on_channel(&mut io, 1), leds_on_channel(&mut io, 2)];
    let channels = sized(&id, found);
    let dev = rows(id, "NZXT Hue+", DeviceKind::Strip, &channels);
    Some(boxed(Box::new(io), dev, FAMILY, HuePlus { channels }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn a_set_length_overrides_the_reported_one() {
        crate::lengths::set_one("hid:nzxt-hue-plus-test", 1, 16);
        assert_eq!(sized("hid:nzxt-hue-plus-test", [24, 40]), [24, 16]);
    }

    #[test]
    fn channel_query() {
        let mut n = 0;
        let ops = record(vec![vec![0xC0, 0, 0, 0x01, 3]], |io| {
            n = leds_on_channel(io, 1)
        });
        assert_eq!(n, 24);
        assert_eq!(
            ops,
            vec![
                Op::Flush(true, false),
                w(vec![0x8D, 1]),
                Op::Sleep(50),
                Op::R(5)
            ]
        );
        assert_eq!(
            record(vec![vec![0, 0, 0, 0, 2]], |io| n = leds_on_channel(io, 2)).len(),
            4
        );
        assert_eq!(n, 20);
    }

    // DeviceUpdateLEDs in Direct: one packet per channel, 5 ms apart
    #[test]
    fn direct_packets() {
        let mut a = vec![Color::BLACK; 3];
        a[0] = Color(1, 2, 3);
        a[2] = Color(4, 5, 6);
        let hue = HuePlus { channels: [2, 1] };
        let [first, _, _] = pushes(hue, 3, vec![], &a, &[]);
        assert_eq!(first[0], w(pad(&[0x4B, 1, 0x0E, 0, 0, 2, 1, 3], PACKET)));
        assert_eq!(first[1], Op::Sleep(5));
        assert_eq!(first[2], w(pad(&[0x4B, 2, 0x0E, 0, 0, 5, 4, 6], PACKET)));
        assert_eq!(first.len(), 4);
    }
}
