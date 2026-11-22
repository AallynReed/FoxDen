// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "evision-v2";
const PAGE: i32 = 0xFF1C;
const ENDORFY_VID: u16 = 0x3299;
const REPORT_ID: u8 = 4;
const CHUNK: usize = 56;
const READ_CAPABILITIES: u8 = 0x03;
const SEND_DYNAMIC_COLORS: u8 = 0x12;
/// OpenRGB blocks on hid_read for each reply; this bounds the wait.
const REPLY_MS: i32 = 1000;

const EVISIONV2_MATRIX: [&[u32]; 6] = [
    &[
        0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, NA, NA, NA, NA,
    ],
    &[
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
    ],
    &[
        37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    ],
    &[
        58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, NA, NA, NA, 72, 73, 74, NA,
    ],
    &[
        75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, NA, 87, NA, 88, NA, 89, 90, 91, 92,
    ],
    &[
        93, 94, 95, NA, NA, NA, 96, NA, NA, NA, 97, 98, 99, 100, 101, 102, 103, NA, 104, 105, NA,
    ],
];

const ENDORFY_MATRIX: [&[u32]; 6] = [
    &[
        0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, NA, NA, NA, NA,
    ],
    &[
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
    ],
    &[
        37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    ],
    &[
        58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, NA, 70, NA, NA, NA, 71, 72, 73, NA,
    ],
    &[
        74, NA, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, NA, NA, 86, NA, 87, 88, 89, 90,
    ],
    &[
        91, 92, 93, NA, NA, NA, 94, NA, NA, NA, 95, 96, 97, 98, 99, 100, 101, 102, NA, 103, NA,
    ],
];

/// Position of each LED's colour in the device's key buffer.
const EVISIONV2_MAP: [u8; 106] = [
    0, 12, 18, 24, 30, 36, 42, 48, 54, 60, 66, 72, 78, 84, 90, 96, 1, 7, 13, 19, 25, 31, 37, 43,
    49, 55, 61, 67, 73, 79, 85, 91, 97, 103, 109, 115, 121, 2, 8, 14, 20, 26, 32, 38, 44, 50, 56,
    62, 68, 74, 80, 86, 92, 98, 104, 110, 116, 122, 3, 9, 15, 21, 27, 33, 39, 45, 51, 57, 63, 69,
    75, 81, 105, 111, 117, 4, 10, 16, 22, 28, 34, 40, 46, 52, 58, 64, 70, 82, 94, 106, 112, 118,
    124, 5, 11, 17, 41, 65, 71, 77, 83, 89, 95, 101, 113, 119,
];

const ENDORFY_MAP: [u8; 104] = [
    0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
    55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 76, 80, 81, 82,
    84, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 97, 99, 101, 102, 103, 104, 105, 106, 107, 111,
    115, 116, 117, 118, 119, 120, 121, 123, 124,
];

fn request(cmd: u8, offset: u16, data: &[u8], size: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = REPORT_ID;
    b[3] = cmd;
    b[4] = size;
    b[5..7].copy_from_slice(&offset.to_le_bytes());
    b[8..8 + data.len()].copy_from_slice(data);
    let sum = b[3..].iter().fold(0u16, |s, &x| s.wrapping_add(x as u16));
    b[1..3].copy_from_slice(&sum.to_le_bytes());
    b
}

/// EVisionV2KeyboardController::Query: the reply echoes the header and carries the data.
fn query(io: &mut dyn Io, cmd: u8, offset: u16, data: &[u8], size: u8) -> Result<Vec<u8>, String> {
    let req = request(cmd, offset, data, size);
    io.write(&req)?;
    let mut r = [0u8; 64];
    let n = loop {
        let n = io.read(&mut r, REPLY_MS)?;
        if n == 0 || r[0] == REPORT_ID {
            break n;
        }
    };
    if n != r.len() || r[0] != REPORT_ID || r[1..4] != req[1..4] || r[5..7] != req[5..7] {
        return Err(format!("evision v2: bad reply to command {cmd:#04x}"));
    }
    if r[7] != 0 {
        return Err(format!("evision v2: command {cmd:#04x} failed ({})", r[7]));
    }
    let size = r[4] as usize;
    if size > CHUNK {
        return Err("evision v2: oversized reply".into());
    }
    Ok(r[8..8 + size].to_vec())
}

/// The constructor's capabilities read; byte 5 is the key buffer size.
pub fn map_size(io: &mut dyn Io) -> Result<usize, String> {
    let (mut caps, mut offset) = (Vec::new(), 0u16);
    while caps.len() < 7 {
        let want = 7 - caps.len();
        let got = query(io, READ_CAPABILITIES, offset, &[], want as u8)?;
        if got.is_empty() || got.len() > want {
            return Err("evision v2: bad capabilities".into());
        }
        offset += got.len() as u16;
        caps.extend(got);
    }
    if caps[0] != 0xAA && caps[1] != 0x55 {
        return Err("evision v2: no capabilities".into());
    }
    Ok(caps[5] as usize)
}

struct EVisionV2 {
    keys: &'static [u8],
    map_size: usize,
}

impl Proto for EVisionV2 {
    /// SetLedsDirect: the whole key buffer in 56-byte dynamic colour writes.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut buf = vec![0u8; 3 * self.map_size];
        for (c, &k) in colors.iter().zip(self.keys) {
            let j = k as usize * 3;
            buf[j..j + 3].copy_from_slice(&rgb(*c));
        }
        for (k, chunk) in buf.chunks(CHUNK).enumerate() {
            let offset = (k * CHUNK) as u16;
            if query(io, SEND_DYNAMIC_COLORS, offset, chunk, chunk.len() as u8)?.is_empty() {
                return Err("evision v2: write not acknowledged".into());
            }
        }
        Ok(())
    }
    /// The keepalive thread refreshes direct mode every 200 ms.
    fn keepalive_ms(&self) -> Option<u64> {
        Some(200)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let (matrix, keys): (&[&[u32]], &'static [u8]) = if info.vid == ENDORFY_VID {
        (&ENDORFY_MATRIX, &ENDORFY_MAP)
    } else {
        (&EVISIONV2_MATRIX, &EVISIONV2_MAP)
    };
    let mut io = opened(api, info)?;
    let map_size = map_size(io.as_mut())?;
    if keys.iter().any(|&k| k as usize >= map_size) {
        return Err(format!("evision v2: key buffer of {map_size} is too small"));
    }
    let dev = mapped(info, FAMILY, d, DeviceKind::Keyboard, matrix, keys.len());
    let proto = EVisionV2 { keys, map_size };
    Ok(Box::new(Driver::new(io, dev, FAMILY, proto)))
}

pub static DETECTORS: &[Detector] = &[
    det(
        "EVision Keyboard 320F:5041",
        0x320F,
        0x5041,
        1,
        PAGE,
        ANY,
        open,
    ),
    det("Endorfy Omnis", ENDORFY_VID, 0x0012, 1, PAGE, ANY, open),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn tables_match_openrgb() {
        for (m, keys) in [
            (&EVISIONV2_MATRIX, &EVISIONV2_MAP[..]),
            (&ENDORFY_MATRIX, &ENDORFY_MAP[..]),
        ] {
            assert!(m.iter().all(|r| r.len() == 21));
            let max = m.iter().flat_map(|r| r.iter()).filter(|&&v| v != NA).max();
            assert_eq!(max, Some(&(keys.len() as u32 - 1)));
        }
        assert_eq!(EVISIONV2_MAP[1], 12);
        assert_eq!(ENDORFY_MAP[103], 124);
    }

    #[test]
    fn capabilities_query() {
        let reply = pad(&[4, 10, 0, 3, 7, 0, 0, 0, 0xAA, 0x55, 0, 0, 0, 126], 64);
        let rec = Rec::with(vec![reply]);
        let mut io = rec.clone();
        assert_eq!(map_size(&mut io), Ok(126));
        assert_eq!(rec.take(), vec![w(pad(&[4, 10, 0, 3, 7], 64)), Op::Read]);
        assert!(map_size(&mut Rec::with(vec![pad(&[4, 9, 0, 3, 7], 64)])).is_err());
    }

    // seven dynamic colour writes cover the 378-byte buffer; each is echoed back
    #[test]
    fn direct_packets() {
        let mut a = vec![Color::BLACK; 106];
        a[0] = Color(1, 2, 3);
        a[1] = Color(4, 5, 6); // key 12: bytes 36..39
        let chunk = |off: u16, size: u8, data: &[(usize, u8)]| {
            let mut p = pad(&[4, 0, 0, 0x12, size, off as u8, (off >> 8) as u8], 64);
            for &(i, v) in data {
                p[8 + i] = v;
            }
            let sum: u16 = p[3..].iter().map(|&x| x as u16).sum();
            p[1] = sum as u8;
            p[2] = (sum >> 8) as u8;
            p
        };
        let mut sent = vec![chunk(
            0,
            56,
            &[(0, 1), (1, 2), (2, 3), (36, 4), (37, 5), (38, 6)],
        )];
        for k in 1..6u16 {
            sent.push(chunk(k * 56, 56, &[]));
        }
        sent.push(chunk(336, 42, &[]));
        assert_eq!(sent[0][1..3], [0x5F, 0]);
        assert_eq!(sent[6][..7], [4, 0x8D, 0, 0x12, 0x2A, 0x50, 0x01]);
        let proto = EVisionV2 {
            keys: &EVISIONV2_MAP,
            map_size: 126,
        };
        let [first, _, _] = pushes(proto, 106, sent.clone(), &a, &a);
        let want: Vec<Op> = sent.into_iter().flat_map(|p| [w(p), Op::Read]).collect();
        assert_eq!(first, want);
    }
}
