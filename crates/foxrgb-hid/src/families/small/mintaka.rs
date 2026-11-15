//! Ported from OpenRGB Controllers/MintakaKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::klm::{Klm, Layout, Op as KeyOp, SIXTY, k};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "mintaka";
const HEADER: u8 = 0x04;
const PAGE: usize = 16;
const MAX_BRIGHTNESS: u8 = 0x0F;
const LIGHTS_OFF: u8 = 0x80;

const OFFSETS: [u32; 62] = [
    19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 103, 37, 38, 39, 40, 41, 42, 43, 44, 45,
    46, 47, 48, 49, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 108, 85, 73, 109, 74, 75, 76,
    77, 78, 79, 80, 81, 82, 83, 84, 91, 92, 93, 94, 95, 96, 97, 98,
];

/// Effect values in mode order; all but lights-off have brightness at its maximum.
const EFFECTS: [u8; 20] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
    0x11, 0x12, 0x13, LIGHTS_OFF,
];

fn keyboard() -> Klm {
    let swap = |row, col, name| k(row, col, 0, name, KeyOp::SwapOnly);
    let iso = [
        swap(1, 11, "'"),
        swap(1, 12, "¿"),
        swap(2, 11, "´"),
        swap(2, 12, "+"),
        swap(3, 10, "Ñ"),
        swap(3, 11, "["),
        swap(3, 12, "]"),
        swap(4, 1, "<"),
        swap(4, 11, "-"),
        swap(5, 11, "Menu"),
        swap(5, 12, "Right Control"),
        swap(5, 13, "Right Fn"),
    ];
    Klm::new(Layout::IsoQwerty, SIXTY, &OFFSETS, &iso)
}

fn send(io: &mut dyn Io, data: &[u8; 64]) -> Result<(), String> {
    let mut b = [0u8; 65];
    b[1..].copy_from_slice(data);
    io.feature(&b)?;
    io.sleep(10);
    Ok(())
}

fn read(io: &mut dyn Io) -> Result<(), String> {
    let mut b = [0u8; 65];
    io.get_feature(&mut b)?;
    io.sleep(10);
    Ok(())
}

fn command(cmd: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = HEADER;
    b[1] = cmd;
    b
}

fn effect_entry(value: u8) -> [u8; PAGE] {
    let mut e = [0u8; PAGE];
    e[0] = value;
    e[9] = if value == LIGHTS_OFF {
        0
    } else {
        MAX_BRIGHTNESS
    };
    e[14] = 0xAA;
    e[15] = 0x55;
    e
}

/// SetMode with Custom active: five pages of effect slots, three blank pages, nine colour pages,
/// then the selected slot (lights-off with Custom's brightness).
pub fn custom_pages(positions: &[u32], colors: &[Color]) -> Vec<[u8; 64]> {
    let mut out = Vec::new();
    let mut selected = effect_entry(LIGHTS_OFF);
    selected[9] = MAX_BRIGHTNESS;
    for page in EFFECTS.chunks(4) {
        let mut b = [0u8; 64];
        for (j, &v) in page.iter().enumerate() {
            let mut e = effect_entry(v);
            if v == LIGHTS_OFF {
                e[9] = MAX_BRIGHTNESS;
            }
            b[j * PAGE..(j + 1) * PAGE].copy_from_slice(&e);
        }
        out.push(b);
    }
    out.extend([[0u8; 64]; 3]);
    let mut colour = [0u8; 576];
    for i in (0..576).step_by(4) {
        colour[i] = 0x80;
    }
    for (&p, c) in positions.iter().zip(colors) {
        let o = p as usize * 4;
        colour[o + 1..o + 4].copy_from_slice(&rgb(*c));
    }
    for chunk in colour.chunks(64) {
        out.push(chunk.try_into().unwrap());
    }
    let mut last = [0u8; 64];
    last[..PAGE].copy_from_slice(&selected);
    out.push(last);
    out
}

/// The Mintaka/Keychron effect-page protocol in Custom mode; LED `i` lands at slot `positions[i]`.
pub struct Pages(pub &'static [u32]);

impl Proto for Pages {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        send(io, &command(0x18))?;
        read(io)?;
        let mut page = command(0x13);
        page[8] = 0x12;
        send(io, &page)?;
        read(io)?;
        for p in custom_pages(self.0, colors) {
            send(io, &p)?;
        }
        send(io, &command(0x02))?;
        read(io)?;
        send(io, &command(0xF0))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    boxed(
        api,
        info,
        FAMILY,
        keyboard().device(info, FAMILY, d),
        Pages(&OFFSETS),
    )
}

pub static DETECTORS: &[Detector] = &[det("VSG Mintaka", 0x05AC, 0x0256, 0, 0x0001, 0x06, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn layout() {
        assert_eq!(keyboard().keys.len(), 62);
    }

    fn f(data: &[u8]) -> Op {
        let mut b = vec![0u8];
        b.extend(pad(data, 64));
        Op::F(b)
    }

    // MintakaKeyboardController::SetMode with the Custom mode active
    #[test]
    fn custom_sequence() {
        let mut a = vec![Color::BLACK; 62];
        a[0] = Color(1, 2, 3); // slot 19
        let [first, _, _] = pushes(Pages(&OFFSETS), 62, vec![], &a, &[]);
        let sends: Vec<&Op> = first.iter().filter(|o| matches!(o, Op::F(_))).collect();
        assert_eq!(sends.len(), 2 + 5 + 3 + 9 + 1 + 2);
        assert_eq!(first.iter().filter(|o| **o == Op::GetF(0)).count(), 3);
        assert_eq!(
            first.iter().filter(|o| **o == Op::Sleep(10)).count(),
            22 + 3
        );
        assert_eq!(first[0], f(&[4, 0x18]));
        assert_eq!(first[2], Op::GetF(0));
        assert_eq!(first[4], f(&[4, 0x13, 0, 0, 0, 0, 0, 0, 0x12]));
        let mut p0 = pad(
            &[1, 0, 0, 0, 0, 0, 0, 0, 0, 15, 0, 0, 0, 0, 0xAA, 0x55, 2],
            64,
        );
        p0[25] = 15;
        p0[30] = 0xAA;
        p0[31] = 0x55;
        p0[32] = 3;
        p0[41] = 15;
        p0[46] = 0xAA;
        p0[47] = 0x55;
        p0[48] = 4;
        p0[57] = 15;
        p0[62] = 0xAA;
        p0[63] = 0x55;
        assert_eq!(*sends[2], f(&p0));
        let Op::F(p4) = sends[6] else { panic!() };
        assert_eq!(
            p4[1 + 48..1 + 64],
            [0x80, 0, 0, 0, 0, 0, 0, 0, 0, 15, 0, 0, 0, 0, 0xAA, 0x55]
        );
        let Op::F(c1) = sends[11] else { panic!() };
        // slot 19 sits at byte 76 of the colour buffer, page 1 offset 12
        assert_eq!(c1[1 + 12..1 + 16], [0x80, 1, 2, 3]);
        assert_eq!(
            *sends[19],
            f(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 15, 0, 0, 0, 0, 0xAA, 0x55])
        );
        assert_eq!(*sends[20], f(&[4, 0x02]));
        assert_eq!(*sends[21], f(&[4, 0xF0]));
        assert_eq!(first.last(), Some(&Op::Sleep(10)));
    }
}
