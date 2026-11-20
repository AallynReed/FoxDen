//! Ported from OpenRGB Controllers/RedragonController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, at, device, on_ip, open_with};
use crate::registry::{Detector, Info};

const VID: u16 = 0x04D9;
const PAGE: u16 = 0xFFA0;
const MODE_STATIC: u8 = 0x02;

/// `RedragonMouseController::SendWritePacket`.
fn write(address: u16, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    b[0] = 0x02;
    b[1] = 0xF3;
    b[2] = address as u8;
    b[3] = (address >> 8) as u8;
    b[4] = data.len() as u8;
    b[8..8 + data.len()].copy_from_slice(data);
    b
}

/// `RedragonMouseController::SendMouseApply`.
fn apply() -> Vec<u8> {
    let mut b = vec![0u8; 16];
    b[..4].copy_from_slice(&[0x02, 0xF1, 0x02, 0x04]);
    b
}

struct Redragon;

impl Protocol for Redragon {
    /// The constructor's profile select, then `DeviceUpdateMode` for Static.
    fn setup(&mut self, colors: &[Color]) -> Vec<Op> {
        let c = at(colors, 0);
        vec![
            Op::Feature(write(0x002C, &[0x00])),
            Op::Feature(apply()),
            Op::Feature(write(0x0449, &[c.0, c.1, c.2, 0x01, 0x00, MODE_STATIC])),
            Op::Feature(apply()),
        ]
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let c = at(colors, 0);
        vec![
            Op::Feature(write(0x0449, &[c.0, c.1, c.2])),
            Op::Feature(apply()),
        ]
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "redragon mouse", |_| {
        let dev = device(
            info.device_id("redragon"),
            d.name,
            DeviceKind::Mouse,
            [1, 1],
            vec![[0.5, 0.5]],
        );
        Ok((dev, Redragon))
    })
}

macro_rules! redragon {
    ($($name:literal $pid:literal),* $(,)?) => {
        pub(super) static DETECTORS: [Detector; 12] = [$(on_ip($name, VID, $pid, 2, PAGE, open)),*];
    };
}

redragon! {
    "Redragon M711 Cobra" 0xFC30,
    "Redragon M715 Dagger" 0xFC39,
    "Redragon M716 Inquisitor" 0xFC3A,
    "Redragon M908 Impact" 0xFC4D,
    "Redragon M602 Griffin" 0xFC38,
    "Redragon M808 Storm" 0xFC5F,
    "Redragon M801 Sniper" 0xFC58,
    "Redragon M810 Taipan" 0xFA7E,
    "Redragon M987 Reaping" 0xFC69,
    "Redragon M921 Azzinoth" 0xFC40,
    "Redragon M711-FPS-1 Cobra FPS" 0xFC62,
    "Redragon M901-K-2 Perdition" 0xFC5E,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(bytes: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b[..bytes.len()].copy_from_slice(bytes);
        b
    }

    // Constructor (SendWritePacket(0x2C, 1, 0) + apply), then DeviceUpdateMode with Static
    #[test]
    fn setup_selects_profile_and_static() {
        let ops = Redragon.setup(&[Color(10, 20, 30)]);
        let apply = report(&[0x02, 0xF1, 0x02, 0x04]);
        assert_eq!(
            ops,
            vec![
                Op::Feature(report(&[0x02, 0xF3, 0x2C, 0x00, 0x01, 0, 0, 0, 0x00])),
                Op::Feature(apply.clone()),
                Op::Feature(report(&[
                    0x02, 0xF3, 0x49, 0x04, 0x06, 0, 0, 0, 10, 20, 30, 0x01, 0x00, 0x02
                ])),
                Op::Feature(apply),
            ]
        );
    }

    // DeviceUpdateLEDs: SendMouseColor + SendMouseApply
    #[test]
    fn frame_sets_colour() {
        let ops = Redragon.frame(&[Color(1, 2, 3)]);
        assert_eq!(
            ops,
            vec![
                Op::Feature(report(&[0x02, 0xF3, 0x49, 0x04, 0x03, 0, 0, 0, 1, 2, 3])),
                Op::Feature(report(&[0x02, 0xF1, 0x02, 0x04])),
            ]
        );
    }
}
