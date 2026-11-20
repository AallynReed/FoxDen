//! Vertagear RGB LED upgrade kits (chair): two 40-LED channels on an NZXT-made controller
//! (protocol learned from SignalRGB's Nzxt/Vertagear_RGB_LED_Upgrade_Kits.js).

use super::{STRIP, boxed, det};
use crate::hid::{Io, Proto};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const CHANNELS: usize = 2;
const CHANNEL_LEDS: usize = 40;
const PER_PACKET: usize = 20;
const LEN: usize = 64;

pub static DETECTORS: &[Detector] = &[det(
    "Vertagear RGB LED Upgrade Kit",
    0x1E71,
    0x2004,
    None,
    None,
    None,
    open,
)];

/// `StreamLightingPacketChanneled`: packet number in the low nibble, channel bit, GRB colours.
pub fn stream(channel: usize, colors: &[Color]) -> Vec<Vec<u8>> {
    colors
        .chunks(PER_PACKET)
        .enumerate()
        .map(|(n, chunk)| {
            let mut b = vec![0x22, 0x10 | n as u8, 1 << channel, 0x00];
            b.extend(chunk.iter().flat_map(|c| [c.1, c.0, c.2]));
            b
        })
        .collect()
}

/// `SubmitLightingColors`.
pub fn submit(channel: usize) -> [u8; 16] {
    [
        0x22,
        0xA0,
        1 << channel,
        0x00,
        0x01,
        0x00,
        0x00,
        0x0A,
        0x00,
        0x00,
        0x80,
        0x00,
        0x32,
        0x00,
        0x00,
        0x01,
    ]
}

struct Kit;

impl Proto for Kit {
    fn frame(&mut self, io: &mut dyn Io, c: &[Color]) -> Result<(), String> {
        for (ch, zone) in c.chunks(CHANNEL_LEDS).enumerate().take(CHANNELS) {
            for p in stream(ch, zone) {
                io.write(&p, LEN)?;
                io.read(LEN, 100)?;
            }
            io.write(&submit(ch), LEN)?;
            io.read(LEN, 100)?;
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); CHANNELS * CHANNEL_LEDS]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = Device::grid(
        &info.device_id("vertagear-kit"),
        d.name,
        DeviceKind::Strip,
        STRIP,
        CHANNEL_LEDS as u32,
        CHANNELS as u32,
    );
    boxed(api, info, "vertagear kit", dev, Kit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grb_packets() {
        let p = stream(1, &[Color(1, 2, 3); 40]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[1][..7], [0x22, 0x11, 0x02, 0x00, 2, 1, 3]);
        assert_eq!(submit(1)[..3], [0x22, 0xA0, 0x02]);
    }
}
