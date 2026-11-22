// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "intel-arc-a770le";
const LOGO: u8 = 0x96;
/// OpenRGB blocks on hid_read for each reply; a bounded wait keeps a silent card from hanging the engine.
const REPLY_MS: i32 = 1000;

fn ids() -> Vec<u8> {
    let fan1 = (0..16).map(|i| 0x01 + 3 * i);
    let fan2 = (0..16).map(|i| 0x31 + 3 * i);
    let back = (0..8).map(|i| 0x02 + 3 * i);
    let ring = (0..50).map(|i| 3 * i);
    fan1.chain(fan2)
        .chain(back)
        .chain(ring)
        .chain([LOGO])
        .collect()
}

fn command(a: u8, b: u8) -> [u8; 65] {
    let mut p = [0u8; 65];
    p[1] = a;
    p[2] = b;
    p
}

pub fn direct_packet(ids: &[u8], colors: &[Color]) -> [u8; 65] {
    let mut p = command(0xC0, 0x01);
    p[3] = ids.len() as u8;
    for (i, (&id, c)) in ids.iter().zip(colors).enumerate() {
        let at = i * 4 + 5;
        p[at] = id;
        if id == LOGO {
            p[at + 1] = c.0.max(c.1).max(c.2);
        } else {
            p[at + 1..at + 4].copy_from_slice(&rgb(*c));
        }
    }
    p
}

fn exchange(io: &mut dyn Io, p: &[u8; 65]) -> Result<(), String> {
    io.write(p)?;
    let mut reply = [0u8; 64];
    io.read(&mut reply, REPLY_MS)?;
    Ok(())
}

struct Arc {
    ids: Vec<u8>,
}

impl Proto for Arc {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        exchange(io, &command(0x41, 0x03))?;
        exchange(io, &command(0x51, 0x28))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (ids, cs) in self.ids.chunks(15).zip(colors.chunks(15)) {
            exchange(io, &direct_packet(ids, cs))?;
        }
        Ok(())
    }
}

fn device(info: &Info, d: &Detector) -> Device {
    let mut dev = Device::grid(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Gpu,
        rect(DeviceKind::Gpu),
        50,
        4,
    );
    let ring = |n: usize, cx: f32| -> Vec<[f32; 2]> {
        (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                [cx + 0.12 * a.sin(), 0.5 - 0.4 * a.cos()]
            })
            .collect()
    };
    let mut leds = ring(16, 0.25);
    leds.extend(ring(16, 0.6));
    leds.extend((0..8).map(|i| [0.1 + 0.8 * i as f32 / 7.0, 0.98]));
    leds.extend((0..50).map(|i| [(i as f32 + 0.5) / 50.0, 0.02]));
    leds.push([0.9, 0.5]);
    dev.leds = leds;
    dev
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    boxed(api, info, FAMILY, device(info, d), Arc { ids: ids() })
}

pub static DETECTORS: &[Detector] = &[det(
    "Intel Arc A770 Limited Edition",
    0x2516,
    0x01B5,
    1,
    0xFF00,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // Enable and apply from the RGBController constructor, then DeviceUpdateLEDs in packets of 15
    #[test]
    fn enable_apply_then_direct() {
        let ids = ids();
        assert_eq!(ids.len(), 91);
        let mut a = vec![Color::BLACK; 91];
        a[0] = Color(1, 2, 3); // fan 1 LED 1, id 0x01
        a[90] = Color(10, 200, 30); // logo
        let [first, _, _] = pushes(Arc { ids }, 91, vec![], &a, &[]);
        assert_eq!(first.len(), 2 * (2 + 7));
        assert_eq!(first[0], w(pad(&[0, 0x41, 0x03], 65)));
        assert_eq!(first[1], Op::Read);
        assert_eq!(first[2], w(pad(&[0, 0x51, 0x28], 65)));
        let Op::W(p0) = &first[4] else { panic!() };
        assert_eq!(p0[..13], [0, 0xC0, 1, 15, 0, 0x01, 1, 2, 3, 0x04, 0, 0, 0]);
        assert_eq!(p0[61..65], [0x2B, 0, 0, 0]);
        assert_eq!(first[16], w(pad(&[0, 0xC0, 1, 1, 0, 0x96, 200, 0, 0], 65)));
    }
}
