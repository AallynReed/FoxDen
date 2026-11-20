//! Ported from OpenRGB Controllers/OKSController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "oks";
const HEAD: u8 = 0x5C;
const WRGB: u8 = 0x15;
const WLED: u8 = 0x17;

/// kb2_port_t header plus data, checksummed as kb2_ComputeChecksum does.
fn pack(cmd: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len() as u8;
    let sum = 0x35u8
        .wrapping_add(HEAD)
        .wrapping_add(len)
        .wrapping_add(cmd)
        .wrapping_add(*data.last().unwrap_or(&0));
    let mut p = vec![HEAD, len, cmd, sum];
    p.extend_from_slice(data);
    p
}

/// kb2M_wrgb sent through Send: one 65-byte write.
pub fn wrgb(bright: u8, mode: u8, speed: u8, dir: u8) -> [u8; 65] {
    let mut data = (bright as u32).to_le_bytes().to_vec();
    data.extend([mode, speed, dir, 0xFF, 0xFF]);
    let p = pack(WRGB, &data);
    let mut b = [0u8; 65];
    b[0] = 0x04;
    b[1..1 + p.len()].copy_from_slice(&p);
    b
}

/// SendColors: 126 slots, 14 per kb2M_wled packet.
pub fn colour_packets(slots: &[Color]) -> Vec<[u8; 65]> {
    (0..126)
        .step_by(14)
        .map(|base| {
            let mut data = Vec::new();
            for (pos, c) in slots.iter().enumerate().skip(base).take(14) {
                data.extend([pos as u8, c.0, c.1, c.2]);
            }
            data.extend([0xFF, 0xFF]);
            let p = pack(WLED, &data);
            let mut b = [0u8; 65];
            b[0] = 0x04;
            b[1..1 + p.len()].copy_from_slice(&p);
            b
        })
        .collect()
}

struct Oks {
    width: usize,
}

impl Proto for Oks {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        // SendInitialize, then Direct: brightness 2, mode 0, speed 2, direction right
        for p in [wrgb(0, 0, 2, 1), wrgb(2, 0x00, 2, 1)] {
            io.write(&p)?;
            io.sleep(2);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut slots = [Color::BLACK; 155];
        for (i, &c) in colors.iter().enumerate() {
            slots[i / self.width * 21 + i % self.width] = c;
        }
        for p in colour_packets(&slots) {
            io.write(&p)?;
        }
        io.sleep(5);
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (width, n) = if info.serial.starts_with('B') {
        (17, 102)
    } else {
        (21, 126)
    };
    let at: Vec<(u32, u32)> = (0..n)
        .map(|i| ((i % width) as u32, (i / width) as u32))
        .collect();
    let dev = cells(info, FAMILY, d, DeviceKind::Keyboard, width as u32, 6, &at);
    boxed(api, info, FAMILY, dev, Oks { width })
}

pub static DETECTORS: &[Detector] = &[det(
    "OKS Optical Axis RGB",
    0x1C4F,
    0xEE88,
    1,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SendInitialize and SendKeyboardModeEx(Direct) via kb2M_wrgb, then SendColors via kb2M_wled
    #[test]
    fn init_mode_then_colours() {
        let mut a = vec![Color::BLACK; 102];
        a[17] = Color(1, 2, 3); // second row on the 87-key board, slot 21
        let [first, _, _] = pushes(Oks { width: 17 }, 102, vec![], &a, &[]);
        // 0x35 + 0x5C + 9 + 0x15 + 0xFF = 0x1AE
        assert_eq!(
            first[0],
            w(pad(
                &[4, 0x5C, 9, 0x15, 0xAE, 0, 0, 0, 0, 0, 2, 1, 0xFF, 0xFF],
                65
            ))
        );
        assert_eq!(
            first[2],
            w(pad(
                &[4, 0x5C, 9, 0x15, 0xAE, 2, 0, 0, 0, 0, 2, 1, 0xFF, 0xFF],
                65
            ))
        );
        assert_eq!(first[1], Op::Sleep(2));
        assert_eq!(first.len(), 4 + 9 + 1);
        let Op::W(p1) = &first[5] else { panic!() };
        // 0x35 + 0x5C + 58 + 0x17 + 0xFF = 0x1E1
        assert_eq!(p1[..5], [4, 0x5C, 58, 0x17, 0xE1]);
        assert_eq!(p1[5..13], [14, 0, 0, 0, 15, 0, 0, 0]);
        assert_eq!(p1[5 + 28..5 + 32], [21, 1, 2, 3]);
        assert_eq!(p1[61..63], [0xFF, 0xFF]);
        assert_eq!(first[13], Op::Sleep(5));
    }
}
