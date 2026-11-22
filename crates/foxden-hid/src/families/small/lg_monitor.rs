// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "lg-monitor";
const LEDS: usize = 48;
const DIRECT: u8 = 0x08;

fn xor(data: &[u8]) -> u8 {
    data.iter().fold(0, |a, &b| a ^ b)
}

/// "SC" command with a two-byte argument, XOR check and "ED" trailer.
fn command(cmd: u8, a: u8, b: u8) -> [u8; 65] {
    let mut p = [0u8; 65];
    p[1..8].copy_from_slice(&[0x53, 0x43, cmd, 0x02, 0x02, a, b]);
    p[8] = xor(&p[1..8]);
    p[9] = 0x45;
    p[10] = 0x44;
    p
}

pub fn turn_on() -> [u8; 65] {
    command(0xCF, 0x01, 0x00)
}

pub fn enable_mode(mode: u8) -> [u8; 65] {
    command(0xCA, 0x03, mode)
}

/// SetDirect's frame split over three 64-byte writes.
pub fn direct_packets(colors: &[Color]) -> [[u8; 65]; 3] {
    let mut data = [0u8; 192];
    data[..6].copy_from_slice(&[0x53, 0x43, 0xC1, 0x02, 0x91, 0x00]);
    let mut at = 6;
    for c in colors.iter().take(LEDS) {
        data[at..at + 3].copy_from_slice(&rgb(*c));
        at += 3;
    }
    data[at] = xor(&data[..at]);
    data[at + 1] = 0x45;
    data[at + 2] = 0x44;
    let mut out = [[0u8; 65]; 3];
    for (i, p) in out.iter_mut().enumerate() {
        p[1..].copy_from_slice(&data[64 * i..64 * (i + 1)]);
    }
    out
}

struct LgMonitor;

impl Proto for LgMonitor {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&turn_on())?;
        io.write(&enable_mode(DIRECT))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        direct_packets(colors).iter().try_for_each(|p| io.write(p))
    }
    // the RGBController's keepalive thread resends direct frames older than 500ms
    fn keepalive_ms(&self) -> Option<u64> {
        Some(500)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info, FAMILY, d, DeviceKind::Strip, LEDS);
    boxed(api, info, FAMILY, dev, LgMonitor)
}

pub static DETECTORS: &[Detector] = &[det(
    "LG 27GN950-B Monitor",
    0x043E,
    0x9A8A,
    1,
    0xFF01,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetDirect: TurnOn and EnableDirectMode the first time, then three data writes
    #[test]
    fn on_direct_then_frame() {
        let mut a = vec![Color::BLACK; LEDS];
        a[0] = Color(1, 2, 3);
        a[47] = Color(4, 5, 6);
        let [first, _, next] = pushes(LgMonitor, LEDS, vec![], &a, &[]);
        assert_eq!(
            first[0],
            w(pad(
                &[0, 0x53, 0x43, 0xCF, 2, 2, 1, 0, 0xDE, 0x45, 0x44],
                65
            ))
        );
        assert_eq!(
            first[1],
            w(pad(
                &[0, 0x53, 0x43, 0xCA, 2, 2, 3, 8, 0xD1, 0x45, 0x44],
                65
            ))
        );
        assert_eq!(
            first[2],
            w(pad(&[0, 0x53, 0x43, 0xC1, 2, 0x91, 0, 1, 2, 3], 65))
        );
        // bytes 147..150 hold LED 47, then the XOR of bytes 0..150 and "ED"
        let check = 0x53 ^ 0x43 ^ 0xC1 ^ 0x02 ^ 0x91 ^ 1 ^ 2 ^ 3 ^ 4 ^ 5 ^ 6;
        let Op::W(p2) = &first[4] else { panic!() };
        assert_eq!(
            p2[1 + 147 - 128..1 + 153 - 128],
            [4, 5, 6, check, 0x45, 0x44]
        );
        assert_eq!(first.len(), 5);
        assert_eq!(next.len(), 3);
    }
}
