// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::kb_layouts::{self as kb, Layout};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-keyboard";
const VID: u16 = 0x0B05;
const REPLY_MS: i32 = 1000;
const CLAYMORE: u16 = 0x184D;
const TUF_K1: u16 = 0x1945;
const TUF_K5: u16 = 0x1899;
const LAYOUT_UK: u8 = 4;
const LAYOUT_US: u8 = 17;

/// The layout table RGBController_AuraKeyboard::SetupZones picks for a product.
fn table(pid: u16, numpad: u8) -> &'static [Layout] {
    match pid {
        0x1875 | 0x18CF | 0x18AF => kb::FLARE,
        0x18F8 | 0x1951 | 0x1B12 | 0x19F6 | 0x19F8 => kb::SCOPE,
        0x190C | 0x1954 | 0x1A05 | 0x19D0 => kb::SCOPE_TKL,
        0x1AB3 | 0x1AB5 => kb::SCOPE_II,
        0x1AAE => kb::SCOPE_II_96,
        0x1B78 => kb::SCOPE_II_96_RX,
        0x19FE => kb::FLARE_II,
        0x19FC => kb::FLARE_II_ANIMATE,
        0x1A83 | 0x1A85 => kb::AZOTH,
        0x193C | 0x193E => kb::FALCHION,
        CLAYMORE => match numpad {
            2 => kb::CLAYMORE_RIGHT,
            3 => kb::CLAYMORE_LEFT,
            _ => kb::CLAYMORE,
        },
        TUF_K1 | TUF_K5 => kb::TUF_K1,
        0x1B30 | 0x1C5E => kb::TUF_K3_GEN2,
        _ => kb::TUF_K7,
    }
}

/// The layout for GetLayout's answer, already cut to a byte.
pub fn pick(table: &'static [Layout], raw: u8) -> Option<&'static Layout> {
    let id = if table.iter().any(|l| l.id == raw % 100) {
        raw % 100
    } else if raw / 100 == 2 {
        LAYOUT_UK
    } else {
        LAYOUT_US
    };
    table.iter().find(|l| l.id == id)
}

/// ClearResponses, a write, then one answer.
fn ask(io: &mut dyn Io, head: &[u8]) -> Result<[u8; 65], String> {
    flush(io)?;
    io.write(&report(head))?;
    let mut r = [0u8; 65];
    io.read(&mut r, REPLY_MS)?;
    Ok(r)
}

/// ClearResponses, a write, then AwaitResponse.
fn send(io: &mut dyn Io, p: &[u8; 65], ms: i32) -> Result<(), String> {
    flush(io)?;
    io.write(p)?;
    let mut r = [0u8; 65];
    io.read(&mut r, ms)?;
    Ok(())
}

fn claymore_layout(release: u16) -> u8 {
    match release >> 12 {
        2 => 204,
        3 => 221,
        _ => 117,
    }
}

/// UpdateLeds: 15 LEDs per packet, each its key code (0 on the K1 and K5) and colour.
pub fn packets(keys: &[u8], per_led: bool, colors: &[Color]) -> Vec<[u8; 65]> {
    keys.chunks(15)
        .zip(colors.chunks(15))
        .map(|(k, c)| {
            let mut b = report(&[0x00, 0xC0, 0x81, k.len() as u8, 0x00]);
            for (j, (key, c)) in k.iter().zip(c).enumerate() {
                b[5 + j * 4] = if per_led { *key } else { 0 };
                b[6 + j * 4..9 + j * 4].copy_from_slice(&rgb(*c));
            }
            b
        })
        .collect()
}

struct Keyboard {
    keys: &'static [u8],
    per_led: bool,
    claymore: bool,
}

impl Proto for Keyboard {
    /// DeviceUpdateMode(Direct): only the Claymore needs remote control allowed.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.claymore {
            send(io, &report(&[0x00, 0x41, 0x01]), 20)?;
            send(io, &report(&[0x00, 0x41, 0x03]), 20)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in packets(self.keys, self.per_led, colors) {
            send(io, &p, 20)?;
        }
        Ok(())
    }
}

/// SetupZones' queries: GetNumpadLocation on the Claymore, GetLayout on the rest.
pub fn layout(io: &mut dyn Io, pid: u16, release: u16) -> Result<&'static Layout, String> {
    let (numpad, raw) = if pid == CLAYMORE {
        let r = ask(io, &[0x00, 0x40, 0x60])?;
        (
            r[5].wrapping_mul(2).wrapping_add(r[4]),
            claymore_layout(release),
        )
    } else {
        let r = ask(io, &[0x00, 0x12, 0x12])?;
        (0, (r[4] as u32 * 100 + r[5] as u32) as u8)
    };
    pick(table(pid, numpad), raw).ok_or_else(|| "no keyboard layout".into())
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let claymore = info.pid == CLAYMORE;
    let layout = layout(io.as_mut(), info.pid, info.release)?;
    let dev = mapped(
        info,
        FAMILY,
        d,
        DeviceKind::Keyboard,
        layout.map,
        layout.keys.len(),
    );
    let proto = Keyboard {
        keys: layout.keys,
        per_led: info.pid != TUF_K1 && info.pid != TUF_K5,
        claymore,
    };
    Ok(Box::new(Driver::new(io, dev, FAMILY, proto)))
}

pub static DETECTORS: &[Detector] = &[
    det(
        "ASUS ROG Strix Scope TKL",
        VID,
        0x190C,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope TKL PNK LTD",
        VID,
        0x1954,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope RX TKL Wireless Deluxe",
        VID,
        0x1A05,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope NX TKL Moonlight White",
        VID,
        0x19D0,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det("ASUS ROG Azoth USB", VID, 0x1A83, 1, 0xFF00, ANY, open),
    det("ASUS ROG Azoth 2.4GHz", VID, 0x1A85, 1, 0xFF00, ANY, open),
    det("ASUS ROG Claymore", VID, CLAYMORE, 1, 0xFF00, ANY, open),
    det(
        "ASUS ROG Falchion (Wired)",
        VID,
        0x193C,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Falchion (Wireless)",
        VID,
        0x193E,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det("ASUS ROG Strix Flare", VID, 0x1875, 1, 0xFF00, ANY, open),
    det(
        "ASUS ROG Strix Flare PNK LTD",
        VID,
        0x18CF,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Flare CoD Black Ops 4 Edition",
        VID,
        0x18AF,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Flare II Animate",
        VID,
        0x19FC,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det("ASUS ROG Strix Flare II", VID, 0x19FE, 1, 0xFF00, ANY, open),
    det("ASUS ROG Strix Scope", VID, 0x18F8, 1, 0xFF00, ANY, open),
    det("ASUS ROG Strix Scope RX", VID, 0x1951, 1, 0xFF00, ANY, open),
    det(
        "ASUS ROG Strix Scope RX EVA-02 Edition",
        VID,
        0x1B12,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope NX Wireless Deluxe USB",
        VID,
        0x19F6,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope NX Wireless Deluxe 2.4GHz",
        VID,
        0x19F8,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det("ASUS ROG Strix Scope II", VID, 0x1AB3, 1, 0xFF00, ANY, open),
    det(
        "ASUS ROG Strix Scope II RX",
        VID,
        0x1AB5,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope II 96 Wireless USB",
        VID,
        0x1AAE,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det(
        "ASUS ROG Strix Scope II 96 RX Wireless USB",
        VID,
        0x1B78,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det("ASUS TUF Gaming K1", VID, TUF_K1, 2, 0xFF00, ANY, open),
    det("ASUS TUF Gaming K3", VID, 0x194B, 1, 0xFF00, ANY, open),
    det(
        "ASUS TUF Gaming K3 GEN II",
        VID,
        0x1B30,
        1,
        0xFF00,
        ANY,
        open,
    ),
    det("ASUS TUF Gaming K5", VID, TUF_K5, 2, 0xFF00, ANY, open),
    det("ASUS TUF Gaming K7", VID, 0x18AA, 1, 0xFF00, ANY, open),
    det(
        "ASUS TUF Gaming K3 GEN II MIKU EDITION",
        VID,
        0x1C5E,
        1,
        0xFF00,
        ANY,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetupZones: the byte-sized layout code, its %100 lookup and the UK/US fallback
    #[test]
    fn layout_choice() {
        assert_eq!(pick(kb::TUF_K7, 117).unwrap().keys.len(), 104);
        assert_eq!(pick(kb::TUF_K7, 204).unwrap().keys.len(), 105);
        assert_eq!(pick(kb::TUF_K7, 209).unwrap().id, LAYOUT_UK);
        assert_eq!(pick(kb::TUF_K7, 105).unwrap().id, LAYOUT_US);
        assert_eq!(pick(kb::SCOPE_II_96, 111).unwrap().id, 11);
        assert!(pick(kb::TUF_K3_GEN2, 204).is_none());
        assert_eq!(claymore_layout(0x2100), 204);
        assert_eq!((3u32 * 100 + 4) as u8, 48);
    }

    // GetLayout and GetNumpadLocation, each after ClearResponses
    #[test]
    fn layout_queries() {
        let mut rec = Rec::with(vec![vec![], pad(&[0, 0x12, 0x12, 0, 2, 4], 65)]);
        let l = layout(&mut rec, 0x18AA, 0).unwrap();
        assert_eq!((l.id, l.keys.len()), (LAYOUT_UK, 105));
        assert_eq!(
            rec.take(),
            vec![Op::Read, w(pad(&[0, 0x12, 0x12], 65)), Op::Read]
        );
        let mut rec = Rec::with(vec![vec![], pad(&[0, 0x40, 0x60, 0, 0, 1], 65)]);
        let l = layout(&mut rec, CLAYMORE, 0x3000).unwrap();
        assert_eq!(
            rec.take(),
            vec![Op::Read, w(pad(&[0, 0x40, 0x60], 65)), Op::Read]
        );
        assert_eq!(l.id, LAYOUT_UK);
        assert!(kb::CLAYMORE_RIGHT.iter().any(|x| std::ptr::eq(x, l)));
    }

    // AuraKeyboardController::UpdateLeds with ClearResponses and AwaitResponse
    #[test]
    fn frame_packets() {
        let layout = pick(kb::TUF_K7, 117).unwrap();
        let n = layout.keys.len();
        let mut a = vec![Color::BLACK; n];
        a[0] = Color(1, 2, 3);
        a[15] = Color(4, 5, 6);
        let proto = Keyboard {
            keys: layout.keys,
            per_led: true,
            claymore: false,
        };
        let [first, _, _] = pushes(proto, n, vec![], &a, &[]);
        assert_eq!(first.len(), n.div_ceil(15) * 3);
        let mut p0 = pad(&[0, 0xC0, 0x81, 15, 0, layout.keys[0], 1, 2, 3], 65);
        for j in 1..15 {
            p0[5 + j * 4] = layout.keys[j];
        }
        assert_eq!(first[..3], [Op::Read, w(p0), Op::Read]);
        let p1 = match &first[4] {
            Op::W(p) => p.clone(),
            _ => panic!(),
        };
        assert_eq!(p1[..9], [0, 0xC0, 0x81, 15, 0, layout.keys[15], 4, 5, 6]);
        let last = match &first[first.len() - 2] {
            Op::W(p) => p.clone(),
            _ => panic!(),
        };
        assert_eq!(last[3] as usize, n - 15 * (n / 15));
    }

    // The Claymore allows remote control before Direct; the K1 sends no key codes
    #[test]
    fn claymore_start_and_k1_codes() {
        let proto = Keyboard {
            keys: kb::CLAYMORE[0].keys,
            per_led: true,
            claymore: true,
        };
        let n = kb::CLAYMORE[0].keys.len();
        let [first, _, _] = pushes(proto, n, vec![], &vec![Color::BLACK; n], &[]);
        assert_eq!(
            first[..6],
            [
                Op::Read,
                w(pad(&[0, 0x41, 1], 65)),
                Op::Read,
                Op::Read,
                w(pad(&[0, 0x41, 3], 65)),
                Op::Read
            ]
        );
        let p = packets(kb::TUF_K1[0].keys, false, &[Color(9, 9, 9); 5]);
        assert_eq!(p[0][..13], [0, 0xC0, 0x81, 5, 0, 0, 9, 9, 9, 0, 9, 9, 9]);
    }
}
