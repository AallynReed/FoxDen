// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "amd-wraith-prism";
/// LED ids: logo, fan, then the ring.
const IDS: [u8; 17] = [
    0x00, 0x01, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A,
    0x09,
];
/// Each reply is awaited; a bounded wait keeps a silent cooler from hanging.
const REPLY_MS: i32 = 1000;

fn packet(head: &[u8]) -> [u8; 65] {
    let mut p = [0u8; 65];
    p[1..1 + head.len()].copy_from_slice(head);
    p
}

pub fn direct_packet(ids: &[u8], colors: &[Color]) -> [u8; 65] {
    let mut p = packet(&[0xC0, 0x01, ids.len() as u8]);
    for (i, (&id, c)) in ids.iter().zip(colors).enumerate() {
        p[5 + i * 4..9 + i * 4].copy_from_slice(&[id, c.0, c.1, c.2]);
    }
    p
}

fn exchange(io: &mut dyn Io, p: &[u8; 65]) -> Result<(), String> {
    io.write(p)?;
    let mut reply = [0u8; 64];
    io.read(&mut reply, REPLY_MS)?;
    Ok(())
}

struct Prism;

impl Proto for Prism {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        exchange(io, &packet(&[0x41, 0x03]))?;
        exchange(io, &packet(&[0x51, 0x28, 0x00, 0x00, 0xE0]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        exchange(io, &direct_packet(&IDS[..2], &colors[..2]))?;
        exchange(io, &direct_packet(&IDS[2..], &colors[2..]))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut dev = ring(info, FAMILY, d, 17);
    dev.leds[0] = [0.5, 0.4];
    dev.leds[1] = [0.5, 0.6];
    for (i, l) in dev.leds[2..].iter_mut().enumerate() {
        let a = i as f32 / 15.0 * std::f32::consts::TAU;
        *l = [0.5 + 0.45 * a.sin(), 0.5 - 0.45 * a.cos()];
    }
    dev.kind = DeviceKind::Other;
    boxed(api, info, FAMILY, dev, Prism)
}

pub static DETECTORS: &[Detector] = &[det(
    "AMD Wraith Prism",
    0x2516,
    0x0051,
    1,
    0xFF00,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DeviceUpdateMode(Direct): SendEnableCommand(true), SendApplyCommand; then two SendDirectPacket
    #[test]
    fn enable_apply_then_direct() {
        let a: Vec<Color> = (0..17).map(|i| Color(i, i, i)).collect();
        let [first, _, _] = pushes(Prism, 17, vec![], &a, &[]);
        assert_eq!(first[0], w(pad(&[0, 0x41, 0x03], 65)));
        assert_eq!(first[1], Op::Read);
        assert_eq!(first[2], w(pad(&[0, 0x51, 0x28, 0, 0, 0xE0], 65)));
        assert_eq!(
            first[4],
            w(pad(&[0, 0xC0, 1, 2, 0, 0, 0, 0, 0, 1, 1, 1, 1], 65))
        );
        let Op::W(r) = &first[6] else { panic!() };
        assert_eq!(r[..13], [0, 0xC0, 1, 15, 0, 0x08, 2, 2, 2, 0x07, 3, 3, 3]);
        assert_eq!(r[61..65], [0x09, 16, 16, 16]);
        assert_eq!(first.len(), 8);
    }
}
