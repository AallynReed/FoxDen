// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Op, Protocol, at, device, on_ip, on_ipu, on_pu, open_with};
use crate::registry::{Detector, Info};

const VID: u16 = 0x0951;
const VID_HP: u16 = 0x03F0;

/// `HyperXPulsefireSurgeController::SendDirect`: 32 ring LEDs as R, G, B planes, then the logo.
fn surge(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[0] = 0x07;
    b[1] = 0x14;
    b[3] = 0xA0;
    for i in 0..32 {
        let c = at(colors, i);
        b[0x08 + i] = c.0;
        b[0x28 + i] = c.1;
        b[0x48 + i] = c.2;
    }
    let logo = at(colors, 32);
    b[0x6C..0x6F].copy_from_slice(&[logo.0, logo.1, logo.2]);
    b
}

/// `HyperXPulsefireFPSProController::SendDirect`.
fn fps_pro(c: Color) -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[..5].copy_from_slice(&[0x07, 0x0A, c.0, c.1, c.2]);
    b[0x08] = 0xA0;
    b
}

/// `HyperXPulsefireHasteController::SendDirectSetup` and `SendDirectColor`.
fn haste(c: Color) -> [Vec<u8>; 2] {
    let mut setup = vec![0u8; 65];
    setup[1] = 0x04;
    setup[2] = 0xF2;
    setup[8] = 0x02;
    let mut color = vec![0u8; 65];
    color[1..5].copy_from_slice(&[0x81, c.0, c.1, c.2]);
    color[8] = 0x02;
    [setup, color]
}

/// `HyperXPulsefireDartController::SendDirect` in static mode at full brightness.
fn dart(c: Color, led: u8) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1..13].copy_from_slice(&[
        0xD2, led, 0x00, 0x08, c.0, c.1, c.2, c.0, c.1, c.2, 0x64, 0x00,
    ]);
    b
}

/// `HyperXPulsefireRaidController::SendColors`.
fn raid(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 264];
    b[0] = 0x07;
    b[1] = 0x0A;
    for i in 0..2 {
        let c = at(colors, i);
        b[2 + i * 3..5 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b[8] = 0xA0;
    b
}

#[derive(Clone, Copy)]
enum Model {
    Surge,
    FpsPro,
    Haste,
    Dart,
    Raid,
}

impl Model {
    fn leds(self) -> usize {
        match self {
            Model::Surge => 33,
            Model::FpsPro | Model::Haste => 1,
            Model::Dart | Model::Raid => 2,
        }
    }
}

struct Mouse(Model);

impl Protocol for Mouse {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        match self.0 {
            Model::Surge => vec![Op::Feature(surge(colors))],
            Model::FpsPro => vec![Op::Feature(fps_pro(at(colors, 0)))],
            Model::Haste => haste(at(colors, 0)).into_iter().map(Op::Feature).collect(),
            Model::Dart => vec![
                Op::Write(dart(at(colors, 0), 0x10)),
                Op::Sleep(2),
                Op::Write(dart(at(colors, 1), 0x00)),
                Op::Sleep(2),
            ],
            Model::Raid => vec![Op::Feature(raid(colors)), Op::Sleep(10)],
        }
    }

    /// The RGBController keepalive threads resend direct colours this often.
    fn keepalive(&self) -> Option<Duration> {
        match self.0 {
            Model::Surge | Model::FpsPro | Model::Haste => Some(Duration::from_millis(50)),
            Model::Raid => Some(Duration::from_secs(1)),
            Model::Dart => None,
        }
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector, model: Model) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "hyperx pulsefire", |_| {
        let n = model.leds();
        let leds = (0..n).map(|i| [0.5, (i as f32 + 0.5) / n as f32]).collect();
        let dev = device(
            info.device_id("hyperx-pulsefire"),
            d.name,
            DeviceKind::Mouse,
            [1, n as u32],
            leds,
        );
        Ok((dev, Mouse(model)))
    })
}

fn open_surge(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Model::Surge)
}

fn open_fps(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Model::FpsPro)
}

fn open_haste(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Model::Haste)
}

fn open_dart(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Model::Dart)
}

fn open_raid(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Model::Raid)
}

pub(super) static DETECTORS: [Detector; 14] = [
    on_ip("HyperX Pulsefire Surge", VID, 0x16D3, 1, 0xFF01, open_surge),
    on_ip(
        "HyperX Pulsefire Surge (HP)",
        VID_HP,
        0x0490,
        1,
        0xFF01,
        open_surge,
    ),
    on_ip("HyperX Pulsefire FPS Pro", VID, 0x16D7, 1, 0xFF01, open_fps),
    on_ip("HyperX Pulsefire Core", VID, 0x16DE, 1, 0xFF01, open_fps),
    on_ip(
        "HyperX Pulsefire Core (HP)",
        VID_HP,
        0x0D8F,
        1,
        0xFF01,
        open_fps,
    ),
    on_ip(
        "HyperX Pulsefire Dart (Wireless)",
        VID,
        0x16E1,
        2,
        0xFF00,
        open_dart,
    ),
    on_ip(
        "HyperX Pulsefire Dart (Wireless)",
        VID_HP,
        0x068E,
        2,
        0xFF00,
        open_dart,
    ),
    on_ip(
        "HyperX Pulsefire Dart (Wired)",
        VID,
        0x16E2,
        1,
        0xFF13,
        open_dart,
    ),
    on_ip(
        "HyperX Pulsefire Dart (Wired)",
        VID_HP,
        0x088E,
        1,
        0xFF13,
        open_dart,
    ),
    on_ipu(
        "HyperX Pulsefire Raid",
        VID,
        0x16E4,
        1,
        0xFF01,
        0x01,
        open_raid,
    ),
    on_ip(
        "HyperX Pulsefire Haste",
        VID_HP,
        0x0F8F,
        3,
        0xFF90,
        open_haste,
    ),
    on_ip("HyperX Pulsefire Haste", VID, 0x1727, 3, 0xFF90, open_haste),
    on_ipu(
        "HyperX Pulsefire Haste",
        VID_HP,
        0x0F8F,
        0,
        0x0001,
        0x01,
        open_haste,
    ),
    on_pu("HyperX Pulsefire Haste", VID, 0x1727, 1, 2, open_haste),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    // HyperXPulsefireSurgeController::SendDirect
    #[test]
    fn surge_planes() {
        let colors: Vec<Color> = (0..33).map(|i| Color(i, 50 + i, 100 + i)).collect();
        let ops = Mouse(Model::Surge).frame(&colors);
        let p = bytes(&ops[0]);
        assert_eq!(p.len(), 264);
        assert_eq!(p[..8], [0x07, 0x14, 0x00, 0xA0, 0, 0, 0, 0]);
        assert_eq!(p[0x08..0x28], (0..32).collect::<Vec<u8>>()[..]);
        assert_eq!(p[0x28..0x48], (50..82).collect::<Vec<u8>>()[..]);
        assert_eq!(p[0x48..0x68], (100..132).collect::<Vec<u8>>()[..]);
        assert_eq!(p[0x68..0x6F], [0, 0, 0, 0, 32, 82, 132]);
        assert!(p[0x6F..].iter().all(|&b| b == 0));
    }

    // HyperXPulsefireFPSProController::SendDirect
    #[test]
    fn fps_pro_packet() {
        let mut want = buf(264, &[0x07, 0x0A, 1, 2, 3]);
        want[8] = 0xA0;
        assert_eq!(
            Mouse(Model::FpsPro).frame(&[Color(1, 2, 3)]),
            vec![Op::Feature(want)]
        );
    }

    // HyperXPulsefireHasteController::SendDirect: setup then colour, every frame
    #[test]
    fn haste_packets() {
        let mut setup = buf(65, &[0x00, 0x04, 0xF2]);
        setup[8] = 0x02;
        let mut color = buf(65, &[0x00, 0x81, 9, 8, 7]);
        color[8] = 0x02;
        assert_eq!(
            Mouse(Model::Haste).frame(&[Color(9, 8, 7)]),
            vec![Op::Feature(setup), Op::Feature(color)]
        );
    }

    // RGBController_HyperXPulsefireDart::DeviceUpdateMode with Direct (static, brightness 100, speed 0)
    #[test]
    fn dart_packets() {
        let ops = Mouse(Model::Dart).frame(&[Color(1, 2, 3), Color(4, 5, 6)]);
        let scroll = buf(
            65,
            &[0x00, 0xD2, 0x10, 0x00, 0x08, 1, 2, 3, 1, 2, 3, 0x64, 0x00],
        );
        let logo = buf(
            65,
            &[0x00, 0xD2, 0x00, 0x00, 0x08, 4, 5, 6, 4, 5, 6, 0x64, 0x00],
        );
        assert_eq!(
            ops,
            vec![
                Op::Write(scroll),
                Op::Sleep(2),
                Op::Write(logo),
                Op::Sleep(2)
            ]
        );
    }

    // HyperXPulsefireRaidController::SendColors + Send's 10 ms pause
    #[test]
    fn raid_packet() {
        let ops = Mouse(Model::Raid).frame(&[Color(1, 2, 3), Color(4, 5, 6)]);
        let want = buf(264, &[0x07, 0x0A, 1, 2, 3, 4, 5, 6, 0xA0]);
        assert_eq!(ops, vec![Op::Feature(want), Op::Sleep(10)]);
    }

    // KeepaliveThread intervals of each RGBController
    #[test]
    fn keepalives() {
        assert_eq!(
            Mouse(Model::Surge).keepalive(),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            Mouse(Model::Haste).keepalive(),
            Some(Duration::from_millis(50))
        );
        assert_eq!(Mouse(Model::Raid).keepalive(), Some(Duration::from_secs(1)));
        assert_eq!(Mouse(Model::Dart).keepalive(), None);
    }
}
