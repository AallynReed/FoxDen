// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "alienware-monitor";
/// Logo, number and power button; 0x0B sets all three.
const LEDS: [u8; 3] = [0x01, 0x02, 0x08];
const ALL: u8 = 0x0B;

fn packet(fill: u8, head: &[u8]) -> [u8; 65] {
    let mut b = [fill; 65];
    b[0] = 0;
    b[1..1 + head.len()].copy_from_slice(head);
    b
}

fn send(io: &mut dyn Io, p: &[u8; 65]) -> Result<(), String> {
    io.write(p)?;
    io.sleep(50);
    Ok(())
}

fn colour(led: u8, c: Color) -> [u8; 65] {
    let mut p = packet(
        0xFF,
        &[
            0x92, 0x37, 0x0A, 0x00, 0x51, 0x87, 0xD0, 0x04, led, c.0, c.1, c.2, 0x64,
        ],
    );
    p[14] = p[5..=13].iter().fold(110, |x, &b| x ^ b);
    p
}

struct AwMonitor;

impl Proto for AwMonitor {
    /// The constructor's Initialize.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        send(io, &packet(0xFF, &[0x95, 0, 0, 0]))?;
        send(
            io,
            &packet(
                0xFF,
                &[
                    0x92, 0x37, 0x08, 0x00, 0x51, 0x85, 0x01, 0xFE, 0x03, 0x00, 0x06, 0x40,
                ],
            ),
        )?;
        send(io, &packet(0x00, &[0x93, 0x37, 0x12]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if colors[0] == colors[1] && colors[1] == colors[2] {
            return send(io, &colour(ALL, colors[0]));
        }
        for (&led, &c) in LEDS.iter().zip(colors) {
            send(io, &colour(led, c))?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Other, LEDS.len());
    boxed(api, info, FAMILY, dev, AwMonitor)
}

pub static DETECTORS: &[Detector] = &[det(
    "Alienware AW3225QF",
    0x187C,
    0x1013,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn filled(head: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF; 65];
        v[..head.len()].copy_from_slice(head);
        v
    }

    #[test]
    fn init_then_colours() {
        let a = vec![Color(1, 2, 3); 3];
        let b = vec![Color(1, 2, 3), Color::BLACK, Color::BLACK];
        let [first, _, next] = pushes(AwMonitor, 3, vec![], &a, &b);
        assert_eq!(first.len(), 8);
        assert_eq!(first[0], w(filled(&[0, 0x95, 0, 0, 0])));
        assert_eq!(first[1], Op::Sleep(50));
        assert_eq!(
            first[2],
            w(filled(&[
                0, 0x92, 0x37, 0x08, 0, 0x51, 0x85, 0x01, 0xFE, 0x03, 0, 0x06, 0x40
            ]))
        );
        assert_eq!(first[4], w(pad(&[0, 0x93, 0x37, 0x12], 65)));
        // all three equal: one write to 0x0B
        assert_eq!(
            first[6],
            w(filled(&[
                0, 0x92, 0x37, 0x0A, 0, 0x51, 0x87, 0xD0, 0x04, 0x0B, 1, 2, 3, 0x64, 0x03
            ]))
        );
        assert_eq!(next.len(), 6);
        assert_eq!(
            next[0],
            w(filled(&[
                0, 0x92, 0x37, 0x0A, 0, 0x51, 0x87, 0xD0, 0x04, 0x01, 1, 2, 3, 0x64, 0x09
            ]))
        );
        let Op::W(power) = &next[4] else { panic!() };
        assert_eq!(power[9..13], [0x08, 0, 0, 0]);
    }
}
