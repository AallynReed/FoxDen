//! Ported from OpenRGB Controllers/FnaticStreakController (GPL-2.0-or-later).

use std::time::Duration;

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "fnatic";
const VID: u16 = 0x2F0E;
const MINI_PID: u16 = 0x0102;
const MINI_SPEED_PID: u16 = 0x0103;
const STREAK65_PID: u16 = 0x0105;

const FULL_MAP: [&[u32]; 6] = [
    &[
        1, 0, 7, 13, 19, 25, 31, 37, 43, 49, 120, 55, 67, 73, 79, 90, 93, 98, 91, 97, 92, 118,
    ],
    &[
        2, 8, 14, 20, 26, 32, 38, 44, 50, 56, 61, 62, 68, NA, 80, 89, 94, 99, 100, 108, 109, 116,
    ],
    &[
        3, 9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69, 75, NA, 81, 88, 95, 96, 101, 107, 110, 115,
    ],
    &[
        4, NA, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 76, 82, NA, NA, NA, 102, 106, 111, NA,
    ],
    &[
        5, 11, 17, 23, 29, 35, 41, 47, 53, 59, 65, 66, NA, 77, NA, NA, 87, NA, 103, 105, 112, 114,
    ],
    &[
        6, 12, 18, NA, NA, NA, 36, NA, NA, NA, 60, 72, NA, 78, 83, 84, 85, 86, 104, NA, 113, NA,
    ],
];

const MINI_MAP: [&[u32]; 6] = [
    &[
        1, 0, 7, 13, 19, 25, 31, 37, 43, 49, 103, 55, 67, 73, 79, 90, 93, 98,
    ],
    &[
        2, 8, 14, 20, 26, 32, 38, 44, 50, 56, 61, 62, 68, NA, 80, 89, 94, 99,
    ],
    &[
        3, 9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69, 75, NA, 81, 88, 95, 96,
    ],
    &[
        4, NA, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 76, 82, NA, NA, NA,
    ],
    &[
        5, 11, 17, 23, 29, 35, 41, 47, 53, 59, 65, 66, NA, 77, NA, NA, 87, NA,
    ],
    &[
        6, 12, 18, NA, NA, NA, 36, NA, NA, NA, 60, 72, NA, 78, 83, 84, 85, 86,
    ],
];

/// FnaticStreakController::SendRequest: 57 data bytes per 65-byte packet behind a length/offset header.
pub fn request(prefix: &[u8], data: &[u8]) -> Vec<[u8; 65]> {
    let all: Vec<u8> = prefix.iter().chain(data).copied().collect();
    let total = all.len();
    all.chunks(57)
        .enumerate()
        .map(|(n, chunk)| {
            let offset = n * 57;
            let mut p = [0u8; 65];
            p[1] = all[0];
            p[2..5].copy_from_slice(&(total as u32).to_le_bytes()[..3]);
            p[5..8].copy_from_slice(&(offset as u32).to_le_bytes()[..3]);
            p[8..8 + chunk.len()].copy_from_slice(chunk);
            p
        })
        .collect()
}

enum Kind {
    Keys { values: Vec<u32>, total: usize },
    Streak65,
}

struct Streak {
    kind: Kind,
}

impl Proto for Streak {
    /// RGBController_FnaticStreak::DeviceUpdateLEDs with Direct at brightness 100.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        match &self.kind {
            Kind::Keys { values, total } => {
                let mut buf = vec![0u8; total * 3];
                for (&v, c) in values.iter().zip(colors) {
                    let v = v as usize;
                    buf[v * 3..v * 3 + 3].copy_from_slice(&rgb(*c));
                }
                for p in request(&[0x0F, 0x03], &buf) {
                    io.write(&p)?;
                }
                Ok(())
            }
            Kind::Streak65 => {
                let mut data = [0u8; 64];
                data[0] = 0x0F;
                data[1] = 0x15;
                data[7] = 0x0F;
                data[8] = 0x03;
                data[9..25].fill(0xFF);
                data[25..28].copy_from_slice(&rgb(colors[0]));
                io.write(&data)
            }
        }
    }

    /// KeepaliveThread: SendKeepalive every 100 ms once Direct has been idle for 500 ms.
    fn keepalive(&self) -> Option<Keepalive> {
        Some(Keepalive {
            idle: Duration::from_millis(500),
            every: Duration::from_millis(100),
        })
    }

    fn refresh(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for p in request(&[0x07], &[]) {
            io.write(&p)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (dev, kind) = if info.pid == STREAK65_PID {
        let dev = placed(
            info,
            FAMILY,
            d,
            DeviceKind::Keyboard,
            [1, 1],
            vec![[0.5, 0.5]],
        );
        (dev, Kind::Streak65)
    } else {
        let (map, total): (&[&[u32]], usize) = if matches!(info.pid, MINI_PID | MINI_SPEED_PID) {
            (&MINI_MAP, 106)
        } else {
            (&FULL_MAP, 124)
        };
        let (values, leds, grid) = mapped(map);
        let dev = placed(info, FAMILY, d, DeviceKind::Keyboard, grid, leds);
        (dev, Kind::Keys { values, total })
    };
    Ok(boxed(opened(api, info)?, FAMILY, dev, Streak { kind }))
}

pub static DETECTORS: [Detector; 4] = [
    det("Fnatic Streak", VID, 0x0101, 1, ANY, ANY, open),
    det("Fnatic miniStreak", VID, MINI_PID, 1, ANY, ANY, open),
    det(
        "Fnatic miniStreak Speed",
        VID,
        MINI_SPEED_PID,
        1,
        ANY,
        ANY,
        open,
    ),
    det("Fnatic Streak65", VID, STREAK65_PID, 1, ANY, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn keys(map: &[&[u32]], total: usize) -> (Streak, usize) {
        let (values, _, _) = mapped(map);
        let n = values.len();
        (
            Streak {
                kind: Kind::Keys { values, total },
            },
            n,
        )
    }

    // FnaticStreakController::SendRequest header layout
    #[test]
    fn request_splits_into_57_byte_chunks() {
        let data: Vec<u8> = (0..100).collect();
        let p = request(&[0x0F, 0x03], &data);
        assert_eq!(p.len(), 2);
        assert_eq!(&p[0][..10], &[0, 0x0F, 102, 0, 0, 0, 0, 0, 0x0F, 0x03]);
        assert_eq!(p[0][64], 54);
        assert_eq!(&p[1][..9], &[0, 0x0F, 102, 0, 0, 57, 0, 0, 55]);
        assert_eq!(p[1][8 + 44], 99);
        assert!(p[1][8 + 45..].iter().all(|&b| b == 0));
    }

    // SetLEDsDirect + SendRGBToDevice for the full Streak: LED value indexes the 372-byte buffer
    #[test]
    fn full_frame() {
        let (proto, n) = keys(&FULL_MAP, 124);
        assert_eq!(n, 112);
        let mut colors = vec![Color::BLACK; n];
        colors[0] = Color(1, 2, 3);
        colors[n - 1] = Color(4, 5, 6);
        let [first, again, _] = pushes(proto, n, vec![], &colors, &colors);
        assert!(again.is_empty());
        let mut buf = vec![0u8; 372];
        buf[0..3].copy_from_slice(&[1, 2, 3]);
        buf[120 * 3..121 * 3].copy_from_slice(&[4, 5, 6]);
        let want: Vec<Op> = request(&[0x0F, 0x03], &buf)
            .into_iter()
            .map(|p| w(p.to_vec()))
            .collect();
        assert_eq!(want.len(), 7);
        assert_eq!(first, want);
    }

    // SendRGBToDevice for the miniStreak: 318 bytes in six packets
    #[test]
    fn mini_frame() {
        let (proto, n) = keys(&MINI_MAP, 106);
        assert_eq!(n, 91);
        let colors = vec![Color(9, 9, 9); n];
        let [first, _, _] = pushes(proto, n, vec![], &colors, &colors);
        assert_eq!(first.len(), 6);
        let Op::W(p0) = &first[0] else { panic!() };
        assert_eq!(&p0[..11], &[0, 0x0F, 64, 1, 0, 0, 0, 0, 0x0F, 0x03, 9]);
    }

    // SendRGBToDevice for FNATIC_STREAK_TYPE_65
    #[test]
    fn streak65_frame() {
        let proto = Streak {
            kind: Kind::Streak65,
        };
        let [first, _, _] = pushes(proto, 1, vec![], &[Color(7, 8, 9)], &[Color(7, 8, 9)]);
        let mut want = pad(&[0x0F, 0x15, 0, 0, 0, 0, 0, 0x0F, 0x03], 64);
        want[9..25].fill(0xFF);
        want[25..28].copy_from_slice(&[7, 8, 9]);
        assert_eq!(first, vec![w(want)]);
    }

    // FnaticStreakController::SendKeepalive
    #[test]
    fn keepalive_packet() {
        let mut rec = Rec::default();
        Streak {
            kind: Kind::Streak65,
        }
        .refresh(&mut rec)
        .unwrap();
        assert_eq!(
            rec.take(),
            vec![w(pad(&[0, 0x07, 1, 0, 0, 0, 0, 0, 0x07], 65))]
        );
    }
}
