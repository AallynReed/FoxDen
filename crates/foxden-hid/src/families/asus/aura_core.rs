// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-core";
const VID: u16 = 0x0B05;
const MODE_STATIC: u8 = 0;
const NO_SPEED: u8 = 0xFF;
const GA15DH_LEDS: usize = 20;
const GA15DH_PER_MESSAGE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Four keyboard zones, 17-byte reports on id 0x5D.
    Keyboard,
    /// A 20-LED strip, 64-byte reports on id 0x5E.
    Ga15dh,
}

impl Kind {
    fn id(self) -> u8 {
        match self {
            Kind::Keyboard => 0x5D,
            Kind::Ga15dh => 0x5E,
        }
    }
    fn size(self) -> usize {
        match self {
            Kind::Keyboard => 17,
            Kind::Ga15dh => 64,
        }
    }
    fn report(self, head: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; self.size()];
        b[0] = self.id();
        b[1..1 + head.len()].copy_from_slice(head);
        b
    }
}

/// IdentifyDevice: a 0x5D feature answer means a keyboard; failing that, 0x5E means the GA15DH.
pub fn identify(io: &mut dyn Io) -> Option<Kind> {
    let mut buf = [0u8; 64];
    buf[0] = 0x5D;
    match io.get_feature(&mut buf) {
        Ok(n) if n > 0 => Some(Kind::Keyboard),
        Ok(_) => None,
        Err(_) => {
            buf[0] = 0x5E;
            matches!(io.get_feature(&mut buf), Ok(n) if n > 0).then_some(Kind::Ga15dh)
        }
    }
}

/// Handshake: the id string and a query, each read back.
pub fn handshake(io: &mut dyn Io) -> Result<(), String> {
    let mut buf = [0u8; 64];
    io.feature(&Kind::Ga15dh.report(b"ASUS Tech.Inc.\0"))?;
    buf[0] = 0x5E;
    io.get_feature(&mut buf)?;
    io.feature(&Kind::Ga15dh.report(&[0x05, 0x20, 0x31, 0x00, 0x10]))?;
    buf[0] = 0x5E;
    io.get_feature(&mut buf)?;
    Ok(())
}

/// InitDirectMode.
pub fn init_direct() -> Vec<Vec<u8>> {
    let mut b = Kind::Ga15dh.report(&[0xBC, 0xD0]);
    let mut out = vec![b.clone()];
    let mut left = GA15DH_LEDS;
    let mut msg = 0;
    while left > 0 {
        b[3..9].copy_from_slice(&[
            0x01,
            0x02,
            0x00,
            msg,
            left.min(GA15DH_PER_MESSAGE) as u8,
            0x00,
        ]);
        out.push(b.clone());
        msg += 1;
        left = left.saturating_sub(GA15DH_PER_MESSAGE);
    }
    out
}

/// UpdateDirect: up to 16 LEDs per report.
pub fn update_direct(colors: &[Color]) -> Vec<Vec<u8>> {
    colors
        .chunks(GA15DH_PER_MESSAGE)
        .enumerate()
        .map(|(msg, chunk)| {
            let mut b = Kind::Ga15dh.report(&[
                0xBC,
                0xD0,
                0x01,
                0x02,
                0x00,
                msg as u8,
                chunk.len() as u8,
                0x00,
            ]);
            for (i, c) in chunk.iter().enumerate() {
                b[9 + i * 3..12 + i * 3].copy_from_slice(&rgb(*c));
            }
            b
        })
        .collect()
}

struct Core(Kind);

impl Proto for Core {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.0 == Kind::Ga15dh {
            for r in init_direct() {
                io.feature(&r)?;
            }
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let k = self.0;
        if k == Kind::Ga15dh {
            for r in update_direct(colors) {
                io.feature(&r)?;
            }
            return Ok(());
        }
        // Static per LED: SendUpdate (zone counts from 1 on keyboards), SendSet, SendApply
        for (led, c) in colors.iter().enumerate() {
            io.feature(&k.report(&[
                0xB3,
                led as u8 + 1,
                MODE_STATIC,
                c.0,
                c.1,
                c.2,
                NO_SPEED,
                0x00,
            ]))?;
            io.feature(&k.report(&[0xB5]))?;
            io.feature(&k.report(&[0xB4]))?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    // Both drivers match 0x1866; one handle per device here, so a known SCAR 17 goes to the laptop driver
    if info.pid == 0x1866 && super::aura_core_laptop::known() {
        return Err("left to the Aura Core laptop driver".into());
    }
    let mut io = opened(api, info)?;
    let kind = identify(io.as_mut()).ok_or("not an Aura Core device")?;
    let dev = match kind {
        Kind::Keyboard => {
            let mut dev = line(info, FAMILY, d, DeviceKind::Keyboard, 4);
            dev.name = "ASUS Aura Keyboard".into();
            dev
        }
        Kind::Ga15dh => {
            handshake(io.as_mut())?;
            let mut dev = line(info, FAMILY, d, DeviceKind::Strip, GA15DH_LEDS);
            dev.name = "ASUS Aura GA15DH".into();
            dev
        }
    };
    Ok(Box::new(Driver::new(io, dev, FAMILY, Core(kind))))
}

pub static DETECTORS: &[Detector] = &[
    det("ASUS Aura Core", VID, 0x1854, ANY, ANY, ANY, open),
    det("ASUS Aura Core", VID, 0x1866, ANY, ANY, ANY, open),
    det("ASUS Aura Core", VID, 0x1869, ANY, ANY, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // IdentifyDevice on a keyboard
    #[test]
    fn identifies_keyboard() {
        let mut rec = Rec::with(vec![vec![0x5D, 1]]);
        assert_eq!(identify(&mut rec), Some(Kind::Keyboard));
        assert_eq!(rec.take(), vec![Op::GetF(0x5D)]);
        assert_eq!(identify(&mut Rec::default()), None);
    }

    // RGBController_AuraCore::DeviceUpdateSingleLED in Static for each keyboard zone
    #[test]
    fn keyboard_frame() {
        let a = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        let [first, _, _] = pushes(Core(Kind::Keyboard), 4, vec![], &a, &[]);
        assert_eq!(first.len(), 12);
        assert_eq!(
            first[0],
            Op::F(pad(&[0x5D, 0xB3, 1, 0, 1, 2, 3, 0xFF, 0], 17))
        );
        assert_eq!(first[1], Op::F(pad(&[0x5D, 0xB5], 17)));
        assert_eq!(first[2], Op::F(pad(&[0x5D, 0xB4], 17)));
        assert_eq!(
            first[9],
            Op::F(pad(&[0x5D, 0xB3, 4, 0, 10, 11, 12, 0xFF, 0], 17))
        );
    }

    // Handshake, InitDirectMode and UpdateDirect on the GA15DH
    #[test]
    fn ga15dh_packets() {
        let mut rec = Rec::default();
        handshake(&mut rec).unwrap();
        let mut id = vec![0x5E];
        id.extend_from_slice(b"ASUS Tech.Inc.");
        assert_eq!(
            rec.take(),
            vec![
                Op::F(pad(&id, 64)),
                Op::GetF(0x5E),
                Op::F(pad(&[0x5E, 0x05, 0x20, 0x31, 0, 0x10], 64)),
                Op::GetF(0x5E),
            ]
        );
        let a: Vec<Color> = (0..20u8).map(|i| Color(i, 0, 0)).collect();
        let [first, _, _] = pushes(Core(Kind::Ga15dh), 20, vec![], &a, &[]);
        assert_eq!(
            first[..3],
            [
                Op::F(pad(&[0x5E, 0xBC, 0xD0], 64)),
                Op::F(pad(&[0x5E, 0xBC, 0xD0, 1, 2, 0, 0, 16, 0], 64)),
                Op::F(pad(&[0x5E, 0xBC, 0xD0, 1, 2, 0, 1, 4, 0], 64)),
            ]
        );
        let mut p0 = pad(&[0x5E, 0xBC, 0xD0, 1, 2, 0, 0, 16, 0], 64);
        for i in 0..16 {
            p0[9 + i * 3] = i as u8;
        }
        assert_eq!(first[3], Op::F(p0));
        assert_eq!(
            first[4],
            Op::F(pad(
                &[
                    0x5E, 0xBC, 0xD0, 1, 2, 0, 1, 4, 0, 16, 0, 0, 17, 0, 0, 18, 0, 0, 19
                ],
                64
            ))
        );
    }
}
