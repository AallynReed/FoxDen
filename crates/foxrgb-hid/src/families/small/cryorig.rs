//! Ported from OpenRGB Controllers/CryorigH7QuadLumiController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "cryorig-h7";
const ZONE_LEDS: usize = 5;
/// OpenRGB loops on blocking reads until the 17-byte reply arrives; this bounds the wait.
const FIRMWARE_TRIES: usize = 10;

pub fn firmware_request() -> [u8; 65] {
    let mut b = [0u8; 65];
    b[0] = 0x02;
    b[1] = 0x5C;
    b
}

/// SetChannelLEDs: fixed mode, GRB data at the channel's offset.
pub fn channel_leds(channel: usize, colors: &[Color]) -> [u8; 65] {
    let mut b = [0u8; 65];
    b[..5].copy_from_slice(&[0x02, 0x4C, channel as u8 + 1, 0x00, 0x00]);
    for (i, c) in colors.iter().enumerate() {
        let at = 5 + channel * 15 + i * 3;
        b[at..at + 3].copy_from_slice(&[c.1, c.0, c.2]);
    }
    b
}

struct Cryorig;

impl Proto for Cryorig {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&firmware_request())?;
        for _ in 0..FIRMWARE_TRIES {
            let mut reply = [0u8; 17];
            if io.read(&mut reply, 1000)? == 17 {
                break;
            }
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (ch, zone) in colors.chunks(ZONE_LEDS).enumerate() {
            io.write(&channel_leds(ch, zone))?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let at: Vec<(u32, u32)> = (0..2).flat_map(|r| (0..5).map(move |c| (c, r))).collect();
    let dev = cells(info, FAMILY, d, DeviceKind::Other, 5, 2, &at);
    boxed(api, info, FAMILY, dev, Cryorig)
}

pub static DETECTORS: &[Detector] = &[det(
    "CRYORIG H7 Quad Lumi",
    0x1E71,
    0x1712,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SendFirmwareRequest from the constructor, then SetChannelLEDs for logo and underglow
    #[test]
    fn firmware_then_channels() {
        let a: Vec<Color> = (0..10).map(|i| Color(i, 10 + i, 20 + i)).collect();
        let [first, _, _] = pushes(Cryorig, 10, vec![vec![0; 17]], &a, &[]);
        assert_eq!(first[0], w(pad(&[2, 0x5C], 65)));
        assert_eq!(first[1], Op::Read);
        let mut c0 = pad(&[2, 0x4C, 1, 0, 0], 65);
        for i in 0..5u8 {
            c0[5 + i as usize * 3..8 + i as usize * 3].copy_from_slice(&[10 + i, i, 20 + i]);
        }
        assert_eq!(first[2], w(c0));
        let Op::W(c1) = &first[3] else { panic!() };
        assert_eq!(c1[..5], [2, 0x4C, 2, 0, 0]);
        assert_eq!(c1[5..20], [0; 15]);
        assert_eq!(c1[20..23], [15, 5, 25]);
        assert_eq!(first.len(), 4);
    }
}
