//! Ported from OpenRGB Controllers/IonicoController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "ionico";
const BAR_PID: u16 = 0x6005;
const BRIGHTNESS_MAX: u8 = 50;

fn report(head: &[u8]) -> [u8; 9] {
    let mut b = [0u8; 9];
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

/// SetMode for Direct: the light bar takes mode 0x33, the keyboard mode 1; speed 0.
pub fn direct_mode(bar: bool) -> [u8; 9] {
    report(&[
        0x08,
        0x02,
        if bar { 0x33 } else { 0x01 },
        0,
        BRIGHTNESS_MAX,
        0x08,
    ])
}

pub enum Out {
    Feature([u8; 9]),
    Write([u8; 65]),
}

/// SetColors without is_mode: one report per keyboard zone, or the bar's RBG frame.
pub fn colours(bar: bool, colors: &[Color]) -> Vec<Out> {
    if !bar {
        return colors
            .iter()
            .enumerate()
            .map(|(i, c)| Out::Feature(report(&[0x14, 0, i as u8 + 1, c.0, c.1, c.2])))
            .collect();
    }
    let mut frame = [0u8; 65];
    // 22 LEDs need 67 bytes; OpenRGB overruns its 65-byte buffer, so the last LED keeps only red here
    for (i, c) in colors.iter().enumerate() {
        for (k, v) in [c.0, c.2, c.1].into_iter().enumerate() {
            if let Some(b) = frame.get_mut(1 + 3 * i + k) {
                *b = v;
            }
        }
    }
    vec![
        Out::Feature(report(&[0x12])),
        Out::Write(frame),
        Out::Feature(report(&[0x12, 0, 0x01])),
    ]
}

struct Ionico {
    bar: bool,
}

impl Proto for Ionico {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&direct_mode(self.bar))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for o in colours(self.bar, colors) {
            match o {
                Out::Feature(b) => io.feature(&b)?,
                Out::Write(b) => io.write(&b)?,
            }
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let bar = info.pid == BAR_PID;
    let (kind, n) = if bar {
        (DeviceKind::Strip, 22)
    } else {
        (DeviceKind::Keyboard, 4)
    };
    let dev = line(info, FAMILY, d, kind, n);
    boxed(api, info, FAMILY, dev, Ionico { bar })
}

pub static DETECTORS: &[Detector] = &[
    det("Ionico Light Bar", 0x048D, 0x6005, ANY, 0xFF03, 0x01, open),
    det("Ionico Keyboard", 0x048D, 0xCE00, ANY, 0xFF12, 0x01, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // DeviceUpdateMode(Direct) then SetColors on the keyboard
    #[test]
    fn keyboard_zones() {
        let a: Vec<Color> = (0..4).map(|i| Color(i, 1, 2)).collect();
        let [first, _, _] = pushes(Ionico { bar: false }, 4, vec![], &a, &[]);
        assert_eq!(
            first[0],
            Op::F(vec![0, 0x08, 0x02, 0x01, 0, 50, 0x08, 0, 0])
        );
        assert_eq!(first[1], Op::F(vec![0, 0x14, 0, 1, 0, 1, 2, 0, 0]));
        assert_eq!(first[4], Op::F(vec![0, 0x14, 0, 4, 3, 1, 2, 0, 0]));
    }

    // DeviceUpdateMode(Direct) then SetColors on the light bar, in R, B, G order
    #[test]
    fn bar_frame() {
        let mut a = vec![Color::BLACK; 22];
        a[0] = Color(1, 2, 3);
        a[21] = Color(4, 5, 6);
        let [first, _, _] = pushes(Ionico { bar: true }, 22, vec![], &a, &[]);
        assert_eq!(
            first[0],
            Op::F(vec![0, 0x08, 0x02, 0x33, 0, 50, 0x08, 0, 0])
        );
        assert_eq!(first[1], Op::F(vec![0, 0x12, 0, 0, 0, 0, 0, 0, 0]));
        let mut f2 = pad(&[0, 1, 3, 2], 65);
        f2[64] = 4;
        assert_eq!(first[2], w(f2));
        assert_eq!(first[3], Op::F(vec![0, 0x12, 0, 1, 0, 0, 0, 0, 0]));
    }
}
