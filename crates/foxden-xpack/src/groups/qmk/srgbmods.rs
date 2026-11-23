//! SRGBmods LED Controller v2 and Wifi LC bridges.

use crate::hid::{Driver, Io, Proto, drain, on_i, open_io, rgb, set_rows, split};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "srgbmods";
const VID: u16 = 0x16D0;
/// Each channel's default length.
const CHANNEL_LEDS: usize = 60;
const FIRMWARE_VERSION: u8 = 0xCC;
const COLORS: u8 = 0xAA;
const WLED_COLORS: u8 = 0xFF;
const NANOLEAF_COLORS: u8 = 0xEE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    V2,
    Wled,
    Nanoleaf,
}

impl Model {
    fn report(self) -> usize {
        match self {
            Model::V2 => 513,
            _ => 65,
        }
    }
    fn per_packet(self) -> usize {
        match self {
            Model::V2 => 169,
            _ => 20,
        }
    }
    fn channels(self) -> usize {
        match self {
            Model::Wled => 4,
            _ => 1,
        }
    }
    /// Packet numbers and totals are one byte, so a channel spans at most 255 packets.
    fn max_leds(self) -> usize {
        255 * self.per_packet()
    }

    /// requestFirmwareVersion: the 0xCC marker after the header's zero bytes.
    fn version_request(self) -> Vec<u8> {
        match self {
            Model::V2 => vec![0, 0, 0, 0, 0, FIRMWARE_VERSION],
            _ => vec![0, 0, 0, 0, FIRMWARE_VERSION],
        }
    }

    /// SendChannel: packets numbered from 1 with the total, then RGB.
    fn packets(self, channel: usize, colors: &[Color]) -> Vec<Vec<u8>> {
        let chunks: Vec<_> = colors.chunks(self.per_packet()).collect();
        let total = chunks.len() as u8;
        chunks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let n = i as u8 + 1;
                let mut p = match self {
                    Model::V2 => vec![0, n, 0, total, 0, COLORS],
                    Model::Wled => vec![0, n, WLED_COLORS, total, channel as u8 + 1],
                    Model::Nanoleaf => vec![0, n, NANOLEAF_COLORS, total, 1],
                };
                p.extend(c.iter().flat_map(|&x| rgb(x)));
                p
            })
            .collect()
    }
}

struct Lc(Model, Vec<usize>);

impl Lc {
    fn all_packets(&self, colors: &[Color]) -> Vec<Vec<u8>> {
        split(colors, &self.1)
            .iter()
            .enumerate()
            .flat_map(|(ch, c)| self.0.packets(ch, c))
            .collect()
    }
}

impl Proto for Lc {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&self.0.version_request(), self.0.report())?;
        let _ = io.read(4, 10);
        drain(io, 4);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in self.all_packets(colors) {
            io.write(&p, self.0.report())?;
        }
        Ok(())
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[]);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let model = match d.pid {
        0x1201 => Model::V2,
        0x113B => Model::Wled,
        _ => Model::Nanoleaf,
    };
    let id = info.device_id(FAMILY);
    let counts: Vec<usize> = (0..model.channels())
        .map(|i| {
            let name = format!("Channel {}", i + 1);
            foxden_hid::zone_length(&id, i, &name, CHANNEL_LEDS, 0, model.max_leds())
        })
        .collect();
    let rect = [0.06, 0.12, 0.52, 0.06 * model.channels() as f32];
    let mut dev = Device::grid(&id, d.name, DeviceKind::Strip, rect, 1, 1);
    set_rows(&mut dev, &counts);
    Ok(Driver::new(open_io(api, info)?, dev, FAMILY, Lc(model, counts)).boxed())
}

pub static DETECTORS: &[Detector] = &[
    on_i("SRGBmods LED Controller v2", VID, 0x1201, 0, open),
    on_i("SRGBmods Wifi LC - WLED", VID, 0x113B, 2, open),
    on_i("SRGBmods Wifi LC - Nanoleaf", VID, 0x1176, 2, open),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_packets_carry_169_leds() {
        let c = vec![Color(1, 2, 3); 200];
        let p = Model::V2.packets(0, &c);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..6], [0, 1, 0, 2, 0, 0xAA]);
        assert_eq!(p[0].len(), 6 + 169 * 3);
        assert!(p[0].len() <= Model::V2.report());
        assert_eq!(p[1][..6], [0, 2, 0, 2, 0, 0xAA]);
        assert_eq!(p[1].len(), 6 + 31 * 3);
    }

    #[test]
    fn channels_follow_their_lengths() {
        assert_eq!(
            (Model::V2.max_leds(), Model::Wled.max_leds()),
            (43095, 5100)
        );
        let c: Vec<Color> = (0..100).map(|i| Color(i, 0, 0)).collect();
        let p = Lc(Model::Wled, vec![25, 0, 60, 15]).all_packets(&c);
        assert_eq!(p.len(), 2 + 3 + 1);
        assert_eq!(p[0][..8], [0, 1, 0xFF, 2, 1, 0, 0, 0]);
        assert_eq!(p[1][..5], [0, 2, 0xFF, 2, 1]);
        assert_eq!(p[2][..8], [0, 1, 0xFF, 3, 3, 25, 0, 0]);
        assert_eq!(p[5][..8], [0, 1, 0xFF, 1, 4, 85, 0, 0]);
        assert_eq!(p[5].len(), 5 + 15 * 3);
        let v2 = Lc(Model::V2, vec![200]).all_packets(&[]);
        assert_eq!((v2.len(), v2[0][3], v2[1].len()), (2, 2, 6 + 31 * 3));
        let full = Lc(Model::Wled, vec![CHANNEL_LEDS; 4]).all_packets(&[]);
        assert_eq!(full.len(), 4 * 3);
    }

    #[test]
    fn wifi_headers() {
        let c = vec![Color(9, 8, 7); 25];
        let w = Model::Wled.packets(2, &c);
        assert_eq!(w[0][..5], [0, 1, 0xFF, 2, 3]);
        assert_eq!(w[0][5..8], [9, 8, 7]);
        assert_eq!(w[0].len(), 65);
        let n = Model::Nanoleaf.packets(0, &c);
        assert_eq!(n[1][..5], [0, 2, 0xEE, 2, 1]);
        assert_eq!(n[1].len(), 5 + 5 * 3);
    }
}
