//! Ported from OpenRGB Controllers/MSIMotherboardController/MSIMotherboard761Controller (GPL-2.0-or-later).

use super::common::{Driver, Op, Proto, device, get_feature, rows};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidDevice;

/// LEDs given to each JAF/JARGB header in place of OpenRGB's user-sized zones.
const HEADER_LEDS: usize = 40;
const SETUP_SIZE: usize = 290;
const FRAME_SIZE: usize = 727;

/// (setup_map index, hdr0, hdr1) of JAF, JARGB1..3, then JPIPE1, in the order Update sends them.
const PACKETS: [(usize, u8, u8); 5] = [
    (3, 0x08, 0x00),
    (0, 0x04, 0x00),
    (1, 0x04, 0x01),
    (2, 0x04, 0x02),
    (4, 0x06, 0x00),
];

/// The constructor's copy of initial_setup_array, before any zone is sized.
fn setup(pipe1: usize) -> [u8; SETUP_SIZE] {
    let mut s = SETUP_761;
    for row in 4..17 {
        for col in [1, 2, 6, 10, 11, 12, 13, 14, 15] {
            s[row * 16 + col] = 0;
        }
    }
    if pipe1 > 0 {
        let row = 4 * 16;
        for (col, v) in [
            (1, 0x09),
            (2, 0xFF),
            (6, 0xFF),
            (10, 0xFF),
            (11, 0xFF),
            (12, 0xFF),
            (13, 0xFF),
            (14, 0x03),
            (15, 0x95),
        ] {
            s[row + col] = v;
        }
    }
    s
}

/// SetZoneLEDCount for JAF and JARGB1..3: the count goes in each row's last byte.
fn sized_setup(pipe1: usize, count: u8) -> [u8; SETUP_SIZE] {
    let mut s = setup(pipe1);
    for (index, _, _) in &PACKETS[..4] {
        s[index * 16 + 16] = count;
    }
    s
}

/// FeaturePacket_PerLED_761 after SetLedColor for one header.
fn frame(hdr0: u8, hdr1: u8, count: u8, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0u8; FRAME_SIZE];
    p[..7].copy_from_slice(&[0x51, 0x09, hdr0, hdr1, 0x00, 0x00, count]);
    for (i, c) in colors.iter().enumerate() {
        p[7 + i * 3..10 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    p
}

struct Gen761 {
    pipe1: usize,
    started: bool,
}

impl Proto for Gen761 {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        if !self.started {
            ops.push(Op::Feature(setup(self.pipe1).to_vec()));
            ops.push(Op::Feature(
                sized_setup(self.pipe1, HEADER_LEDS as u8).to_vec(),
            ));
            self.started = true;
        }
        for (k, &(_, hdr0, hdr1)) in PACKETS.iter().enumerate() {
            let (start, count) = if k < 4 {
                (k * HEADER_LEDS, HEADER_LEDS)
            } else if self.pipe1 > 0 {
                (4 * HEADER_LEDS, self.pipe1)
            } else {
                continue;
            };
            let end = (start + count).min(colors.len());
            let slice = colors.get(start..end).unwrap_or(&[]);
            ops.push(Op::Feature(frame(hdr0, hdr1, count as u8, slice)));
        }
        ops
    }

    fn reset(&mut self) {
        self.started = false;
    }
}

/// The SMBIOS baseboard product, which OpenRGB's DMIInfo prefixes with "MSI ".
#[cfg(windows)]
fn board_product() -> Option<String> {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            len: *mut u32,
        ) -> i32;
    }
    let hklm = 0x8000_0002u32 as i32 as isize as *mut c_void;
    let sub: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\BIOS\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "BaseBoardProduct\0".encode_utf16().collect();
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: both names are NUL-terminated and `len` is the byte size of `buf`.
    let status = unsafe {
        RegGetValueW(
            hklm,
            sub.as_ptr(),
            value.as_ptr(),
            0x2,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != 0 {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..n]);
    Some(text.trim_end_matches('\0').trim().to_string())
}

#[cfg(not(windows))]
fn board_product() -> Option<String> {
    None
}

fn pipe1_for(board: &str) -> Option<usize> {
    BOARDS_761.iter().find(|b| b.0 == board).map(|b| b.1)
}

fn layout(pipe1: usize) -> (Vec<[f32; 2]>, [u32; 2]) {
    let mut zones = vec![HEADER_LEDS; 4];
    if pipe1 > 0 {
        zones.push(pipe1);
    }
    rows(&zones)
}

/// The 761-byte branch of DetectMSIMotherboardControllers; the board is checked before the probe write.
pub fn open(hid: HidDevice, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let board = format!(
        "MSI {}",
        board_product().ok_or("cannot read the board name")?
    );
    let pipe1 = pipe1_for(&board).ok_or_else(|| format!("{board} has no 761-byte config"))?;
    get_feature(&hid, 0x50, SETUP_SIZE)?;
    let mut probe = vec![0u8; 761];
    probe[0] = 0x51;
    hid.send_feature_report(&probe).map_err(|e| e.to_string())?;
    get_feature(&hid, 0x51, 500)?;
    let dev = device(
        info.device_id("msi-mystic"),
        det.name,
        DeviceKind::Motherboard,
        layout(pipe1),
    );
    Ok(Driver::boxed(
        hid,
        "msi mystic light",
        dev,
        Gen761 {
            pipe1,
            started: false,
        },
    ))
}

// (board name, pipe1 LEDs); pipe1 > 0 means zone_set2 with JPIPE1
const BOARDS_761: &[(&str, usize)] = &[
    ("MSI MAG X870 TOMAHAWK WIFI (MS-7E51)", 0),
    ("MSI MAG B850M MORTAR WIFI (MS-7E61)", 0),
    ("MSI MPG B850I EDGE TI WIFI (MS-7E79)", 0),
    ("MSI X870 GAMING PLUS WIFI (MS-7E47)", 0),
    ("MSI B850 GAMING PLUS WIFI6E (MS-7E80)", 0),
    ("MSI B850M GAMING PLUS WIFI6E (MS-7E81)", 0),
    ("MSI MPG X870E CARBON WIFI (MS-7E49)", 0),
    ("MSI Z890 GAMING PLUS WIFI (MS-7E34)", 0),
    ("MSI Z890 GAMING PLUS WIFI6E (MS-7E34)", 0),
    ("MSI X870E GAMING PLUS WIFI (MS-7E70)", 0),
    ("MSI MAG X870E TOMAHAWK WIFI (MS-7E59)", 0),
    ("MSI PRO B850-P WIFI (MS-7E56)", 0),
    ("MSI B850M GAMING PLUS WIFI (MS-7E66)", 0),
    ("MSI PRO X870E-P WIFI (MS-7E70)", 0),
    ("MSI MPG X870I EDGE TI EVO WIFI (MS-7E50)", 0),
    ("MSI B850 GAMING PLUS WIFI (MS-7E56)", 0),
    ("MSI PRO X870-P WIFI (MS-7E47)", 0),
    ("MSI MPG X870E EDGE TI WIFI (MS-7E59)", 10),
    ("MSI MAG B850 TOMAHAWK MAX WIFI (MS-7E62)", 0),
    ("MSI PRO B850M-P WIFI (MS-7E71)", 0),
    ("MSI MAG Z890 TOMAHAWK WIFI (MS-7E32)", 0),
    ("MSI MPG B850 EDGE TI WIFI (MS-7E62)", 0),
    ("MSI PRO B850M-VC WIFI6E (MS-7E71)", 0),
    ("MSI MAG B850 TOMAHAWK WIFI (MS-7E53)", 0),
    ("MSI MEG Z890 UNIFY-X (MS-7E20)", 0),
    ("MSI PRO X870E-S EVO WIFI (MS-7E86)", 0),
    ("MSI MAG X870E GAMING MAX WIFI (MS-7E86)", 0),
    ("MSI PRO Z890-P WIFI (MS-7E34)", 0),
    ("MSI B850 GAMING PLUS WIFI PZ (MS-7E75)", 0),
    ("MSI MAG X870E TOMAHAWK MAX WIFI PZ (MS-7E84)", 0),
    ("MSI PRO B850M-A WIFI PZ (MS-7E78)", 0),
    ("MSI B850MPOWER (MS-7E83)", 0),
    ("MSI B840M GAMING WIFI6E (MS-7E76)", 0),
    ("MSI PRO B850M-A WIFI (MS-7E66)", 0),
    ("MSI PRO B850-S WIFI6E (MS-7E80)", 0),
    ("MSI B850M GAMING WIFI (MS-7E82)", 0),
    ("MSI PRO B850-S EVO WIFI (MS-7E97)", 0),
    ("MSI B850M GAMING WIFI6E (MS-7E87)", 0),
];

#[rustfmt::skip]
const SETUP_761: [u8; 290] = [
    0x50,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x15, 0x78,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x15, 0x78,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x15, 0x78,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x15, 0x78,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x95, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x95, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x95, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x1E,
    0x09, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x95, 0x1E,
    0x00,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(op: &Op) -> &[u8] {
        match op {
            Op::Feature(b) => b,
            other => panic!("{other:?}"),
        }
    }

    // MSIMotherboard761Controller constructor, zone_set1: rows 4..16 cleared, the 0x1E tail kept.
    #[test]
    fn setup_zone_set1() {
        let s = setup(0);
        assert_eq!(s[0], 0x50);
        assert_eq!(s[1..17], SETUP_761[1..17]);
        assert_eq!(
            s[49..65],
            [
                0x09, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x15, 0x78
            ]
        );
        for row in 4..17 {
            let mut e = [0u8; 16];
            e[15] = 0x1E;
            assert_eq!(s[row * 16 + 1..row * 16 + 17], e, "row {row}");
        }
        assert_eq!(s[273..290], SETUP_761[273..290]);
    }

    // zone_set2 (MPG X870E EDGE TI): JPIPE1 re-armed after the clear.
    #[test]
    fn setup_zone_set2() {
        let s = setup(10);
        assert_eq!(
            s[65..81],
            [
                0x09, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x95, 0x1E
            ]
        );
        assert_eq!(
            s[81..97],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x1E]
        );
    }

    // SetZoneLEDCount: ARRAY_ROW(zone, 16) for JARGB1..3 and JAF.
    #[test]
    fn sized_setup_counts() {
        let s = sized_setup(0, 40);
        assert_eq!((s[16], s[32], s[48], s[64]), (40, 40, 40, 40));
        let mut rest = s;
        for at in [16, 32, 48, 64] {
            rest[at] = setup(0)[at];
        }
        assert_eq!(rest, setup(0));
    }

    // First Update: both setup arrays, then JAF, JARGB1..3 (pipe1 is 0, so JPIPE1 is never sent).
    #[test]
    fn first_push_order() {
        let mut g = Gen761 {
            pipe1: 0,
            started: false,
        };
        let colors: Vec<Color> = (0..160).map(|i| Color(i as u8, 0, 1)).collect();
        let ops = g.frame(&colors);
        assert_eq!(ops.len(), 6);
        assert_eq!(bytes(&ops[0]), &setup(0)[..]);
        assert_eq!(bytes(&ops[1]), &sized_setup(0, 40)[..]);
        let jaf = bytes(&ops[2]);
        assert_eq!(jaf.len(), 727);
        assert_eq!(jaf[..10], [0x51, 0x09, 0x08, 0x00, 0, 0, 40, 0, 0, 1]);
        assert_eq!(jaf[7 + 39 * 3..7 + 40 * 3], [39, 0, 1]);
        assert!(jaf[7 + 40 * 3..].iter().all(|&b| b == 0));
        assert_eq!(
            bytes(&ops[3])[..10],
            [0x51, 0x09, 0x04, 0x00, 0, 0, 40, 40, 0, 1]
        );
        assert_eq!(bytes(&ops[4])[..7], [0x51, 0x09, 0x04, 0x01, 0, 0, 40]);
        assert_eq!(
            bytes(&ops[5])[..10],
            [0x51, 0x09, 0x04, 0x02, 0, 0, 40, 120, 0, 1]
        );
        assert_eq!(g.frame(&colors).len(), 4);
    }

    // JPIPE1 frame on the EDGE TI: hdr 06/00, count numof_pipe1_leds.
    #[test]
    fn pipe1_frame() {
        let mut g = Gen761 {
            pipe1: 10,
            started: true,
        };
        let mut colors = vec![Color::BLACK; 170];
        colors[160] = Color(7, 8, 9);
        let ops = g.frame(&colors);
        assert_eq!(ops.len(), 5);
        assert_eq!(
            bytes(&ops[4])[..10],
            [0x51, 0x09, 0x06, 0x00, 0, 0, 10, 7, 8, 9]
        );
        assert_eq!(layout(10).0.len(), 170);
    }

    #[test]
    fn board_lookup() {
        assert_eq!(pipe1_for("MSI MPG X870E EDGE TI WIFI (MS-7E59)"), Some(10));
        assert_eq!(pipe1_for("MSI MAG X870 TOMAHAWK WIFI (MS-7E51)"), Some(0));
        assert_eq!(pipe1_for("MSI B760M GAMING PLUS WIFI (MS-7D99)"), None);
        assert_eq!(BOARDS_761.len(), 38);
    }
}
