//! Ported from OpenRGB Controllers/SRGBmodsController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, any, at, backend, device, open_io, rows};
use crate::registry::{Detector, Info};

/// LEDs given to each header in place of OpenRGB's user-sized zones, which start at 0.
const CHANNEL_LEDS: usize = 60;
const PER_PACKET: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    Pico,
    V1,
}

/// `DetectSRGBmodsControllers`: the model from the product string.
fn model(product: &str) -> Option<Model> {
    match product {
        "SRGBmods Pico LED Controller" | "Pico LED Controller" => Some(Model::Pico),
        "LED Controller v1" => Some(Model::V1),
        _ => None,
    }
}

impl Model {
    fn channels(self) -> usize {
        match self {
            Model::Pico => 2,
            Model::V1 => 1,
        }
    }

    /// `SRGBmodsPicoController::SendPacket` / `SRGBmodsLEDControllerV1::SendPacket`.
    fn packet(self, channel: u8, this: u8, last: u8, colors: &[Color]) -> Vec<u8> {
        let mut b = vec![0u8; 65];
        let head = match self {
            Model::Pico => [0, this, 0, last, channel + 1],
            Model::V1 => [0, this, last, 0, 0xAA],
        };
        b[..5].copy_from_slice(&head);
        for (i, c) in colors.iter().enumerate() {
            b[5 + i * 3..8 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
        }
        b
    }

    /// `SetChannelLEDs`: packets of 20 colours numbered from 1.
    fn channel(self, channel: u8, colors: &[Color]) -> Vec<Vec<u8>> {
        let n = colors.chunks(PER_PACKET).len() as u8;
        colors
            .chunks(PER_PACKET)
            .enumerate()
            .map(|(i, chunk)| self.packet(channel, i as u8 + 1, n, chunk))
            .collect()
    }
}

struct Srgb {
    model: Model,
}

impl Driver for Srgb {
    // RGBController_SRGBmodsLEDControllerV1::DeviceUpdateMode(Direct): SetDirect, then 200 ms.
    fn setup(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.model == Model::V1 {
            let mut b = vec![0u8; 65];
            b[4] = 0xBB;
            io.write(&b)?;
            io.sleep(Duration::from_millis(200));
        }
        Ok(())
    }

    // DeviceUpdateLEDs of both models: every channel in turn.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for ch in 0..self.model.channels() {
            let zone: Vec<Color> = (0..CHANNEL_LEDS)
                .map(|i| at(colors, ch * CHANNEL_LEDS + i))
                .collect();
            for p in self.model.channel(ch as u8, &zone) {
                io.write(&p)?;
            }
        }
        Ok(())
    }

    // SRGBmodsPicoController::KeepaliveThread: checks every 5 s for a second without a commit.
    fn keepalive(&self) -> Option<Duration> {
        (self.model == Model::Pico).then_some(Duration::from_secs(5))
    }

    fn refresh(&mut self, io: &mut dyn Io, _last: &[Color]) -> Result<(), String> {
        io.write(&self.model.packet(1, 0, 0, &[]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let model = model(&info.product).ok_or("unknown SRGBmods product")?;
    let io = open_io(api, info)?;
    let (leds, grid) = rows(&vec![CHANNEL_LEDS; model.channels()]);
    let dev = device(
        info.device_id("srgbmods"),
        d.name,
        DeviceKind::Strip,
        grid,
        leds,
    );
    Ok(backend(io, dev, Srgb { model }, "srgbmods"))
}

pub(super) static DETECTORS: [Detector; 2] = [
    any("SRGBmods Pico LED Controller", 0x16D0, 0x1123, open),
    any("SRGBMods LED Controller v1", 0x16D0, 0x1205, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op, buf, ms};
    use super::*;

    // DetectSRGBmodsControllers: product strings
    #[test]
    fn model_from_product() {
        assert_eq!(model("Pico LED Controller"), Some(Model::Pico));
        assert_eq!(model("SRGBmods Pico LED Controller"), Some(Model::Pico));
        assert_eq!(model("LED Controller v1"), Some(Model::V1));
        assert_eq!(model("Other"), None);
    }

    fn colors(n: usize) -> Vec<Color> {
        (0..n).map(|i| Color(i as u8, 1, 2)).collect()
    }

    // SRGBmodsPicoController::SetChannelLEDs for both headers
    #[test]
    fn pico_frame() {
        let mut fake = Fake::default();
        let mut s = Srgb { model: Model::Pico };
        s.setup(&mut fake).unwrap();
        s.frame(&mut fake, &colors(120)).unwrap();
        let ops = fake.ops();
        assert_eq!(ops.len(), 6);
        let mut p = buf(65, &[0, 3, 0, 3, 2]);
        for i in 0..20 {
            p[5 + i * 3..8 + i * 3].copy_from_slice(&[100 + i as u8, 1, 2]);
        }
        assert_eq!(ops[5], Op::Write(p));
        let Op::Write(first) = &ops[0] else { panic!() };
        assert_eq!(first[..8], [0, 1, 0, 3, 1, 0, 1, 2]);
    }

    // SRGBmodsPicoController::KeepaliveThread: SendPacket(1, 0, 0, false, NULL, 0)
    #[test]
    fn pico_keepalive() {
        let mut fake = Fake::default();
        let mut s = Srgb { model: Model::Pico };
        assert_eq!(s.keepalive(), Some(Duration::from_secs(5)));
        s.refresh(&mut fake, &[]).unwrap();
        assert_eq!(fake.ops(), vec![Op::Write(buf(65, &[0, 0, 0, 0, 2]))]);
    }

    // SRGBmodsLEDControllerV1::SetDirect, then SetChannelLEDs
    #[test]
    fn v1_setup_and_frame() {
        let mut fake = Fake::default();
        let mut s = Srgb { model: Model::V1 };
        assert_eq!(s.keepalive(), None);
        s.setup(&mut fake).unwrap();
        s.frame(&mut fake, &colors(60)).unwrap();
        let ops = fake.ops();
        assert_eq!(ops[..2], [Op::Write(buf(65, &[0, 0, 0, 0, 0xBB])), ms(200)]);
        assert_eq!(ops.len(), 5);
        let Op::Write(p) = &ops[3] else { panic!() };
        assert_eq!(p[..8], [0, 2, 3, 0, 0xAA, 20, 1, 2]);
    }
}
