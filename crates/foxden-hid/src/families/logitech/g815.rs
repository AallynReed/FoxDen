// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::keys::{self, G_KEYS, INDICATORS, LOGO, MEDIA_KEYS, MODIFIER_KEYS, NA};
use super::{FAMILY, Hid, ack, matrix_leds, open_sibling};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 300;
const LITTLE: u8 = 0x1F;
const BIG: u8 = 0x6F;
const COMMIT: u8 = 0x7F;
const LED_COUNT: usize = 117;

#[rustfmt::skip]
const MATRIX: [[u8; 27]; 7] = [
    [110, NA, NA, NA, NA, NA, NA, NA, NA, NA, 111, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA],
    [NA, NA, 37, NA, 54, 55, 56, 57, NA, 58, 59, 60, 61, NA, 62, 63, 64, 65, NA, 66, 67, 68, NA, 106, 107, 108, 109],
    [112, NA, 49, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, NA, 41, 42, 38, NA, NA, 69, 70, 71, NA, 79, 80, 81, 82],
    [113, NA, 39, NA, 16, 22, 4, 17, NA, 19, 24, 20, 8, 14, 15, 43, 44, 45, NA, 72, 73, 74, NA, 91, 92, 93, 83],
    [114, NA, 53, NA, 0, 18, 3, 5, NA, 6, 7, 9, 10, 11, 47, 48, 46, 36, NA, NA, NA, NA, NA, 88, 89, 90, NA],
    [115, NA, 99, 96, 25, 23, 2, 21, NA, 1, NA, 13, 12, 50, 51, 52, 103, NA, NA, NA, 78, NA, NA, 85, 86, 87, 84],
    [116, NA, 98, 101, 100, NA, NA, NA, NA, 40, NA, NA, NA, NA, 104, 105, 97, 102, NA, 76, 77, 75, NA, 94, NA, 95, NA],
];

/// (zone, index) of each LED in OpenRGB's led_names order.
fn leds() -> Vec<(u8, u8)> {
    let mut v: Vec<_> = keys::usages(0x04, 0x65).collect();
    v.extend(MODIFIER_KEYS);
    v.extend(MEDIA_KEYS);
    v.push((LOGO, 0x01));
    v.push((INDICATORS, 0x99));
    v.extend(G_KEYS);
    v
}

fn report(kind: u8, data: &[u8]) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..4].copy_from_slice(&[0x11, 0xFF, 0x10, kind]);
    r[4..4 + data.len()].copy_from_slice(data);
    r
}

/// InitializeDirect, then SendSingleLed(0x29, 0, 0, 0) and Commit, as DeviceUpdateMode sends for Direct.
pub fn direct_setup() -> Vec<[u8; 20]> {
    let mut a = [0u8; 20];
    a[..4].copy_from_slice(&[0x11, 0xFF, 0x08, 0x3E]);
    let mut b = a;
    b[3] = 0x1E;
    let mut c = [0u8; 20];
    c[..4].copy_from_slice(&[0x11, 0xFF, 0x0F, 0x1E]);
    c[0x10] = 0x01;
    let mut d = c;
    d[4] = 0x01;
    vec![
        a,
        b,
        c,
        d,
        report(LITTLE, &[0x29, 0, 0, 0]),
        report(COMMIT, &[]),
    ]
}

/// DeviceUpdateLEDs: one big frame per 13 keys for colours on more than 4 keys, little frames of
/// 4 key-colour pairs for the rest, then a commit.
pub fn frames(groups: &[(Color, Vec<u8>)]) -> Vec<[u8; 20]> {
    let mut out = Vec::new();
    let mut little = [0u8; 16];
    let mut n = 0;
    for (c, codes) in groups {
        if codes.len() > 4 {
            for chunk in codes.chunks(13) {
                let mut big = [0u8; 16];
                big[..3].copy_from_slice(&[c.0, c.1, c.2]);
                big[3..3 + chunk.len()].copy_from_slice(chunk);
                if 3 + chunk.len() < 16 {
                    big[3 + chunk.len()] = 0xFF;
                }
                out.push(report(BIG, &big));
            }
        } else {
            for &code in codes {
                little[n * 4..n * 4 + 4].copy_from_slice(&[code, c.0, c.1, c.2]);
                n += 1;
                if n == 4 {
                    out.push(report(LITTLE, &little));
                    n = 0;
                }
            }
        }
    }
    if n > 0 {
        little[n * 4..].fill(0);
        little[n * 4] = 0xFF;
        out.push(report(LITTLE, &little));
    }
    if !groups.is_empty() {
        out.push(report(COMMIT, &[]));
    }
    out
}

pub struct G815 {
    hid: Box<dyn Hid>,
    /// The 64-byte collection; OpenRGB opens it with the 20-byte one but never writes to it.
    _wide: Box<dyn Hid>,
    device: Device,
    leds: Vec<(u8, u8)>,
    sent: Option<Vec<Color>>,
}

impl G815 {
    fn new(hid: Box<dyn Hid>, wide: Box<dyn Hid>, id: String, name: &str) -> Self {
        let device = Device {
            id,
            name: name.into(),
            kind: DeviceKind::Keyboard,
            rect: [0.06, 0.58, 0.54, 0.32],
            grid: [27, 7],
            leds: matrix_leds(&MATRIX, LED_COUNT),
        };
        G815 {
            hid,
            _wide: wide,
            device,
            leds: leds(),
            sent: None,
        }
    }

    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        if self.sent.is_none() {
            for r in direct_setup() {
                ack(self.hid.as_ref(), &r, TIMEOUT)?;
            }
        }
        let groups = keys::by_color(&self.leds, self.sent.as_deref(), colors);
        for r in frames(&groups) {
            ack(self.hid.as_ref(), &r, TIMEOUT)?;
        }
        Ok(())
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = open_sibling(api, info, 0xFF43, 0x0602)?;
    let wide = open_sibling(api, info, 0xFF43, 0x0604)?;
    Ok(Box::new(G815::new(
        Box::new(hid),
        Box::new(wide),
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G815 {
    fn name(&self) -> &str {
        "logitech g815"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if self.sent.as_deref() == Some(colors) {
            return Ok(());
        }
        match self.send(colors) {
            Ok(()) => {
                self.sent = Some(colors.to_vec());
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, take};
    use super::*;

    fn hex(s: &str) -> [u8; 20] {
        let mut r = [0u8; 20];
        for (i, b) in s.split_whitespace().enumerate() {
            r[i] = u8::from_str_radix(b, 16).unwrap();
        }
        r
    }

    // InitializeDirect, SendSingleLed and SendCommit in LogitechG815Controller.cpp.
    #[test]
    fn direct_setup_matches_openrgb() {
        assert_eq!(
            direct_setup(),
            vec![
                hex("11 FF 08 3E"),
                hex("11 FF 08 1E"),
                hex("11 FF 0F 1E 00 00 00 00 00 00 00 00 00 00 00 00 01"),
                hex("11 FF 0F 1E 01 00 00 00 00 00 00 00 00 00 00 00 01"),
                hex("11 FF 10 1F 29"),
                hex("11 FF 10 7F"),
            ]
        );
    }

    // RGBController_LogitechG815::DeviceUpdateLEDs: big frames go out as built, little ones every 4 keys.
    #[test]
    fn frames_match_openrgb_packing() {
        let red = Color(0xFF, 0, 0);
        let blue = Color(0, 0, 0xFF);
        let codes: Vec<u8> = (1..=17).collect();
        let groups = vec![(red, vec![0x26, 0x27]), (blue, codes)];
        let f = frames(&groups);
        assert_eq!(f.len(), 4);
        // red has 2 keys: they wait in the little frame while blue's 17 keys go out as two big frames
        assert_eq!(
            f[0],
            hex("11 FF 10 6F 00 00 FF 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D")
        );
        assert_eq!(f[1], hex("11 FF 10 6F 00 00 FF 0E 0F 10 11 FF"));
        // C++ zeroes from 4n-1 and so loses the last key's blue byte; this keeps it
        assert_eq!(f[2], hex("11 FF 10 1F 26 FF 00 00 27 FF 00 00 FF"));
        assert_eq!(f[3], hex("11 FF 10 7F"));
    }

    #[test]
    fn full_little_frame_has_no_terminator() {
        let groups = vec![
            (Color(1, 2, 3), vec![0x10, 0x11]),
            (Color(4, 5, 6), vec![0x12, 0x13]),
        ];
        let f = frames(&groups);
        assert_eq!(
            f[0],
            hex("11 FF 10 1F 10 01 02 03 11 01 02 03 12 04 05 06 13 04 05 06")
        );
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn every_led_sits_once_in_the_matrix() {
        let mut seen = [0; LED_COUNT];
        for row in MATRIX {
            for led in row {
                if led != NA {
                    seen[led as usize] += 1;
                }
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
        assert_eq!(leds().len(), LED_COUNT);
        assert_eq!(leds()[37], (keys::KEYBOARD, 0x29));
        assert_eq!(leds()[110], (LOGO, 0x01));
    }

    #[test]
    fn push_sets_up_once_and_skips_repeats() {
        let (hid, log) = Fake::silent();
        let (wide, wide_log) = Fake::silent();
        let mut kb = G815::new(Box::new(hid), Box::new(wide), "k".into(), "G815");
        let mut colors = vec![Color(0, 0, 0xFF); LED_COUNT];
        kb.push("k", &colors).unwrap();
        let first = take(&log);
        assert_eq!(
            first[..6].to_vec(),
            direct_setup()
                .iter()
                .map(|r| r.to_vec())
                .collect::<Vec<_>>()
        );
        // 117 keys of one colour: nine big frames and a commit
        assert_eq!(first.len(), 6 + 9 + 1);
        kb.push("k", &colors).unwrap();
        assert!(take(&log).is_empty());
        colors[37] = Color(1, 2, 3);
        kb.push("k", &colors).unwrap();
        assert_eq!(
            take(&log),
            vec![
                hex("11 FF 10 1F 26 01 02 03 FF").to_vec(),
                hex("11 FF 10 7F").to_vec()
            ]
        );
        assert!(take(&wide_log).is_empty());
    }
}
