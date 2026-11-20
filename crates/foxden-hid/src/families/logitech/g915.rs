//! Ported from OpenRGB Controllers/LogitechController/LogitechG915Controller (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidApi;

use super::keys::{self, G_KEYS, INDICATORS, LOGO, MEDIA_KEYS, MODIFIER_KEYS, NA};
use super::{FAMILY, Hid, ack, matrix_leds};
use crate::registry::{Detector, Info};

const TIMEOUT: i32 = 300;
const LITTLE: u8 = 0x1F;
const BIG: u8 = 0x6F;
const COMMIT: u8 = 0x7F;
const FULL_LEDS: usize = 117;
const TKL_LEDS: usize = 95;

#[rustfmt::skip]
const MATRIX: [[u8; 27]; 7] = [
    [93, NA, NA, NA, NA, NA, NA, NA, NA, NA, 94, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA, NA],
    [NA, NA, 37, NA, 54, 55, 56, 57, NA, 58, 59, 60, 61, NA, 62, 63, 64, 65, NA, 66, 67, 68, NA, 89, 90, 91, 92],
    [112, NA, 49, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, NA, 41, 42, 38, NA, NA, 69, 70, 71, NA, 95, 96, 97, 98],
    [113, NA, 39, NA, 16, 22, 4, 17, NA, 19, 24, 20, 8, 14, 15, 43, 44, 45, NA, 72, 73, 74, NA, 107, 108, 109, 99],
    [114, NA, 53, NA, 0, 18, 3, 5, NA, 6, 7, 9, 10, 11, 47, 48, 46, 36, NA, NA, NA, NA, NA, 104, 105, 106, NA],
    [115, NA, 82, 79, 25, 23, 2, 21, NA, 1, NA, 13, 12, 50, 51, 52, 86, NA, NA, NA, 78, NA, NA, 101, 102, 103, 100],
    [116, NA, 81, 84, 83, NA, NA, NA, NA, 40, NA, NA, NA, NA, 87, 88, 80, 85, NA, 76, 77, 75, NA, 110, NA, 111, NA],
];

#[rustfmt::skip]
const MATRIX_TKL: [[u8; 20]; 7] = [
    [93, NA, NA, NA, NA, 94, NA, NA, NA, NA, NA, NA, 89, 90, 91, 92, NA, NA, NA, NA],
    [37, NA, 54, 55, 56, 57, NA, 58, 59, 60, 61, NA, 62, 63, 64, 65, NA, 66, 67, 68],
    [49, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, NA, 41, 42, 38, NA, NA, 69, 70, 71],
    [39, NA, 16, 22, 4, 17, NA, 19, 24, 20, 8, 14, 15, 43, 44, 45, NA, 72, 73, 74],
    [53, NA, 0, 18, 3, 5, NA, 6, 7, 9, 10, 11, 47, 48, 46, 36, NA, NA, NA, NA],
    [82, 79, 25, 23, 2, 21, NA, 1, NA, 13, 12, 50, 51, 52, 86, NA, NA, NA, 78, NA],
    [81, 84, 83, NA, NA, NA, NA, 40, NA, NA, NA, NA, 87, 88, 80, 85, NA, 76, 77, 75],
];

/// (zone, index) of each LED in OpenRGB's led_names order; a TKL takes the first 95.
fn leds(tkl: bool) -> Vec<(u8, u8)> {
    let mut v: Vec<_> = keys::usages(0x04, 0x52).collect();
    v.extend(keys::usages(0x64, 0x65));
    v.extend(MODIFIER_KEYS);
    v.extend(MEDIA_KEYS);
    v.push((LOGO, 0x01));
    v.push((INDICATORS, 0x99));
    v.extend(keys::usages(0x53, 0x63));
    v.extend(G_KEYS);
    v.truncate(if tkl { TKL_LEDS } else { FULL_LEDS });
    v
}

/// Device index and feature indices (4522, 8071, 8081) the constructor hard-codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indices {
    pub device: u8,
    pub f4522: u8,
    pub f8071: u8,
    pub f8081: u8,
}

pub fn indices(wired: bool) -> Indices {
    if wired {
        Indices {
            device: 0xFF,
            f4522: 0x0E,
            f8071: 0x09,
            f8081: 0x0A,
        }
    } else {
        Indices {
            device: 0x01,
            f4522: 0x0F,
            f8071: 0x0A,
            f8081: 0x0B,
        }
    }
}

fn report(device: u8, feature: u8, function: u8, data: &[u8]) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..4].copy_from_slice(&[0x11, device, feature, function]);
    r[4..4 + data.len()].copy_from_slice(data);
    r
}

/// InitializeDirect, SendSingleLed(0x29, 0, 0, 0) and Commit, as DeviceUpdateMode sends for Direct.
pub fn direct_setup(ix: Indices) -> Vec<[u8; 20]> {
    let mut c = report(ix.device, ix.f8071, 0x1E, &[]);
    c[0x10] = 0x01;
    let mut d = c;
    d[4] = 0x01;
    vec![
        report(ix.device, ix.f4522, 0x3E, &[]),
        report(ix.device, ix.f4522, 0x1E, &[]),
        c,
        d,
        report(ix.device, ix.f8081, LITTLE, &[0x29, 0, 0, 0]),
        report(ix.device, ix.f8081, COMMIT, &[]),
    ]
}

/// DeviceUpdateLEDs: per 13-key chunk, a big frame if more than 4 keys are left, else little
/// pairs; all big frames, then all little frames, then a commit.
pub fn frames(ix: Indices, groups: &[(Color, Vec<u8>)]) -> Vec<[u8; 20]> {
    if groups.is_empty() {
        return Vec::new();
    }
    let mut big = Vec::new();
    let mut pairs = Vec::new();
    for (c, codes) in groups {
        for chunk in codes.chunks(13) {
            if chunk.len() > 4 {
                let mut buf = vec![c.0, c.1, c.2];
                buf.extend_from_slice(chunk);
                if buf.len() < 16 {
                    buf.push(0xFF);
                }
                big.push(report(ix.device, ix.f8081, BIG, &buf));
            } else {
                pairs.extend(chunk.iter().map(|&k| [k, c.0, c.1, c.2]));
            }
        }
    }
    let mut out = big;
    for frame in pairs.chunks(4) {
        let mut buf = frame.concat();
        if frame.len() < 4 {
            buf.push(0xFF);
        }
        out.push(report(ix.device, ix.f8081, LITTLE, &buf));
    }
    out.push(report(ix.device, ix.f8081, COMMIT, &[]));
    out
}

pub struct G915 {
    hid: Box<dyn Hid>,
    ix: Indices,
    device: Device,
    leds: Vec<(u8, u8)>,
    sent: Option<Vec<Color>>,
}

impl G915 {
    fn new(hid: Box<dyn Hid>, wired: bool, tkl: bool, id: String, name: &str) -> Self {
        let (grid, leds) = if tkl {
            ([20, 7], matrix_leds(&MATRIX_TKL, TKL_LEDS))
        } else {
            ([27, 7], matrix_leds(&MATRIX, FULL_LEDS))
        };
        let device = Device {
            id,
            name: name.into(),
            kind: DeviceKind::Keyboard,
            rect: [0.06, 0.58, if tkl { 0.44 } else { 0.54 }, 0.32],
            grid,
            leds,
        };
        G915 {
            hid,
            ix: indices(wired),
            device,
            leds: self::leds(tkl),
            sent: None,
        }
    }

    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        if self.sent.is_none() {
            for r in direct_setup(self.ix) {
                ack(self.hid.as_ref(), &r, TIMEOUT)?;
            }
        }
        let groups = keys::by_color(&self.leds, self.sent.as_deref(), colors);
        for r in frames(self.ix, &groups) {
            ack(self.hid.as_ref(), &r, TIMEOUT)?;
        }
        Ok(())
    }
}

pub fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let wired = matches!(info.pid, 0xC33E | 0xC343);
    let tkl = matches!(info.pid, 0xC545 | 0xC343);
    let hid = info.open(api)?;
    Ok(Box::new(G915::new(
        Box::new(hid),
        wired,
        tkl,
        info.device_id(FAMILY),
        det.name,
    )))
}

impl Backend for G915 {
    fn name(&self) -> &str {
        "logitech g915"
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

    fn hex(s: &str) -> Vec<u8> {
        let mut r = vec![0u8; 20];
        for (i, b) in s.split_whitespace().enumerate() {
            r[i] = u8::from_str_radix(b, 16).unwrap();
        }
        r
    }

    fn v(f: Vec<[u8; 20]>) -> Vec<Vec<u8>> {
        f.iter().map(|r| r.to_vec()).collect()
    }

    // InitializeDirect, SendSingleLed and SendCommit in LogitechG915Controller.cpp, receiver indices.
    #[test]
    fn direct_setup_matches_openrgb_receiver() {
        assert_eq!(
            v(direct_setup(indices(false))),
            vec![
                hex("11 01 0F 3E"),
                hex("11 01 0F 1E"),
                hex("11 01 0A 1E 00 00 00 00 00 00 00 00 00 00 00 00 01"),
                hex("11 01 0A 1E 01 00 00 00 00 00 00 00 00 00 00 00 01"),
                hex("11 01 0B 1F 29"),
                hex("11 01 0B 7F"),
            ]
        );
    }

    // Same sequence on a cable: LogitechG915Controller's wired constructor branch.
    #[test]
    fn direct_setup_matches_openrgb_wired() {
        let s = v(direct_setup(indices(true)));
        assert_eq!(s[0], hex("11 FF 0E 3E"));
        assert_eq!(
            s[2],
            hex("11 FF 09 1E 00 00 00 00 00 00 00 00 00 00 00 00 01")
        );
        assert_eq!(s[4], hex("11 FF 0A 1F 29"));
        assert_eq!(s[5], hex("11 FF 0A 7F"));
    }

    // RGBController_LogitechG915::DeviceUpdateLEDs with populate_big/little_frame_data.
    #[test]
    fn frames_match_openrgb_packing() {
        let red = Color(0xFF, 0, 0);
        let blue = Color(0, 0, 0xFF);
        let codes: Vec<u8> = (1..=17).collect();
        let groups = vec![(red, vec![0x26, 0x27]), (blue, codes)];
        let f = v(frames(indices(false), &groups));
        assert_eq!(
            f,
            vec![
                hex("11 01 0B 6F 00 00 FF 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D"),
                hex("11 01 0B 1F 26 FF 00 00 27 FF 00 00 0E 00 00 FF 0F 00 00 FF"),
                hex("11 01 0B 1F 10 00 00 FF 11 00 00 FF FF"),
                hex("11 01 0B 7F"),
            ]
        );
        assert!(frames(indices(false), &[]).is_empty());
    }

    #[test]
    fn big_frame_short_of_13_keys_is_terminated() {
        let f = v(frames(
            indices(true),
            &[(Color(1, 2, 3), vec![1, 2, 3, 4, 5])],
        ));
        assert_eq!(f[0], hex("11 FF 0A 6F 01 02 03 01 02 03 04 05 FF"));
    }

    #[test]
    fn every_led_sits_once_in_each_matrix() {
        fn check<const C: usize>(m: &[[u8; C]], count: usize) {
            let mut seen = vec![0; count];
            for row in m {
                for &led in row {
                    if led != NA {
                        seen[led as usize] += 1;
                    }
                }
            }
            assert!(seen.iter().all(|&n| n == 1));
        }
        check(&MATRIX, FULL_LEDS);
        check(&MATRIX_TKL, TKL_LEDS);
        assert_eq!(leds(false)[79], (keys::KEYBOARD, 0x64));
        assert_eq!(leds(false)[95], (keys::KEYBOARD, 0x53));
        assert_eq!(leds(false)[116], (keys::GKEYS, 5));
        assert_eq!(leds(true).len(), TKL_LEDS);
    }

    #[test]
    fn push_sets_up_once_and_skips_repeats() {
        let (hid, log) = Fake::silent();
        let mut kb = G915::new(Box::new(hid), false, true, "k".into(), "G915 TKL");
        let mut colors = vec![Color::BLACK; TKL_LEDS];
        kb.push("k", &colors).unwrap();
        let first = take(&log);
        assert_eq!(first[..6].to_vec(), v(direct_setup(indices(false))));
        // 95 black keys: seven big frames of 13, then four keys in a little frame
        assert_eq!(first.len(), 6 + 7 + 1 + 1);
        kb.push("k", &colors).unwrap();
        assert!(take(&log).is_empty());
        colors[93] = Color(9, 8, 7);
        kb.push("k", &colors).unwrap();
        assert_eq!(
            take(&log),
            vec![hex("11 01 0B 1F D2 09 08 07 FF"), hex("11 01 0B 7F")]
        );
    }
}
