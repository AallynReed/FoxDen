// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "qmk-vial";
pub const LEN: usize = 33;

/// SendCommand: report id 0, `head`, `args`; the reply is read over the same buffer.
pub fn send(io: &mut dyn Io, head: &[u8], args: &[u8]) -> Result<([u8; LEN], usize), String> {
    let mut b = [0u8; LEN];
    b[1..1 + head.len()].copy_from_slice(head);
    b[1 + head.len()..1 + head.len() + args.len()].copy_from_slice(args);
    io.write(&b)?;
    let n = io.read(&mut b[..LEN - 1], 1000)?;
    Ok((b, n))
}

/// rgb2hsv with the hue scaled to 0..256, as the direct packets carry it.
pub fn hsv(c: Color) -> [u8; 3] {
    let (r, g, b) = (c.0 as i32, c.1 as i32, c.2 as i32);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    if d == 0 {
        return [0, 0, max as u8];
    }
    let mut h = if r == max {
        (g - b) * 60 / d
    } else if g == max {
        (b - r) * 60 / d + 120
    } else {
        (r - g) * 60 / d + 240
    };
    if h < 0 {
        h += 360;
    }
    [
        (h as f32 * (256.0f32 / 360.0)) as u8,
        ((256 * d - 8) / max) as u8,
        max as u8,
    ]
}

/// SetupZones: each LED at its key-matrix (row, col); LEDs without one sit in the first cell.
pub fn matrix(id: &str, name: &str, at: &[Option<(u8, u8)>]) -> Device {
    let rows = at
        .iter()
        .flatten()
        .map(|p| p.0 as u32 + 1)
        .max()
        .unwrap_or(1);
    let cols = at
        .iter()
        .flatten()
        .map(|p| p.1 as u32 + 1)
        .max()
        .unwrap_or(1);
    let kind = DeviceKind::Keyboard;
    let mut dev = Device::grid(id, name, kind, rect(kind), cols, rows);
    dev.leds = at
        .iter()
        .map(|p| {
            let (r, c) = p.unwrap_or((0, 0));
            [
                (c as f32 + 0.5) / cols as f32,
                (r as f32 + 0.5) / rows as f32,
            ]
        })
        .collect();
    dev
}

/// The constructor's queries; None unless the board speaks VialRGB.
fn probe(io: &mut dyn Io) -> Option<Vec<Option<(u8, u8)>>> {
    let (b, _) = send(io, &[0x01, 0x00], &[]).ok()?;
    if u16::from_le_bytes([b[2], b[3]]) < 9 {
        return None;
    }
    let (b, _) = send(io, &[0xFE, 0x00], &[]).ok()?;
    if u32::from_le_bytes([b[0], b[1], b[2], b[3]]) < 4 || b[12] & 1 == 0 {
        return None;
    }
    let (b, _) = send(io, &[0x08, 0x43], &[]).ok()?;
    let n = u16::from_le_bytes([b[2], b[3]]);
    let at = (0..n)
        .map(|i| {
            let (b, _) = send(io, &[0x08, 0x44], &i.to_le_bytes()).ok()?;
            Some((b[5] != 0xFF && b[6] != 0xFF).then_some((b[5], b[6])))
        })
        .collect::<Option<Vec<_>>>()?;
    (!at.is_empty()).then_some(at)
}

struct Vial;

impl Proto for Vial {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        send(io, &[0x07, 0x41], &[1, 0, 0, 0, 0, 0]).map(|_| ())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (i, run) in colors.chunks(9).enumerate() {
            let mut a = [0u8; 30];
            a[..2].copy_from_slice(&((i * 9) as u16).to_le_bytes());
            a[2] = run.len() as u8;
            for (k, c) in run.iter().enumerate() {
                a[3 + k * 3..6 + k * 3].copy_from_slice(&hsv(*c));
            }
            send(io, &[0x07, 0x42], &a)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let at = probe(io.as_mut()).ok_or("no VialRGB")?;
    let name = if info.product.is_empty() {
        d.name
    } else {
        &info.product
    };
    let dev = matrix(&info.device_id(FAMILY), name, &at);
    Ok(Box::new(Driver::new(io, dev, FAMILY, Vial)))
}

pub static DETECTORS: &[Detector] = &[
    det("Raspberry Pi 500+", 0x2E8A, 0x0011, ANY, 0xFF60, 0x61, open),
    det("YMDK 84", 0x20A0, 0x422D, ANY, 0xFF60, 0x61, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // QMKVialRGBController constructor: VIA version, Vial id, LED count, then info per LED
    #[test]
    fn probe_reads_leds() {
        let mut rec = Rec::with(vec![
            pad(&[0x01, 0x00, 0x09], 32),
            pad(&[4, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0x01], 32),
            pad(&[0x08, 0x43, 3, 0], 32),
            pad(&[0x08, 0x44, 10, 20, 4, 0, 2], 32),
            pad(&[0x08, 0x44, 30, 40, 4, 1, 0], 32),
            pad(&[0x08, 0x44, 0, 0, 0, 0xFF, 0xFF], 32),
        ]);
        let at = probe(&mut rec).unwrap();
        assert_eq!(at, [Some((0, 2)), Some((1, 0)), None]);
        let ops = rec.take();
        let sent: Vec<Op> = [
            &[0, 0x01, 0x00][..],
            &[0, 0xFE, 0x00],
            &[0, 0x08, 0x43],
            &[0, 0x08, 0x44, 0, 0],
            &[0, 0x08, 0x44, 1, 0],
            &[0, 0x08, 0x44, 2, 0],
        ]
        .iter()
        .flat_map(|h| [w(pad(h, 33)), Op::Read])
        .collect();
        assert_eq!(ops, sent);
        let dev = matrix("t", "t", &at);
        assert_eq!(dev.grid, [3, 2]);
        assert_eq!(dev.leds[0], [2.5 / 3.0, 0.25]);
    }

    #[test]
    fn old_via_is_not_vial() {
        let mut rec = Rec::with(vec![pad(&[0x01, 0x00, 0x08], 32)]);
        assert!(probe(&mut rec).is_none());
        assert_eq!(rec.take().len(), 2);
    }

    // SetMode(VIALRGB_EFFECT_DIRECT), then CmdSendLEDs nine LEDs a packet in HSV
    #[test]
    fn direct_then_packets() {
        let mut a = vec![Color::BLACK; 10];
        a[0] = Color(255, 0, 0);
        a[1] = Color(0, 255, 0);
        a[2] = Color(10, 20, 30);
        a[3] = Color(100, 100, 100);
        a[9] = Color(0, 0, 128);
        let mut b = a.clone();
        b[9] = Color::BLACK;
        let [first, _, next] = pushes(Vial, 10, vec![], &a, &b);
        let p0 = pad(
            &[
                0, 0x07, 0x42, 0, 0, 9, 0, 255, 255, 85, 255, 255, 149, 170, 30, 0, 0, 100,
            ],
            33,
        );
        let p1 = pad(&[0, 0x07, 0x42, 9, 0, 1, 170, 255, 128], 33);
        assert_eq!(
            first,
            [
                w(pad(&[0, 0x07, 0x41, 1], 33)),
                Op::Read,
                w(p0.clone()),
                Op::Read,
                w(p1),
                Op::Read,
            ]
        );
        assert_eq!(
            next,
            [
                w(p0),
                Op::Read,
                w(pad(&[0, 0x07, 0x42, 9, 0, 1], 33)),
                Op::Read
            ]
        );
    }
}
