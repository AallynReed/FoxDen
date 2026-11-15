//! Ported from OpenRGB Controllers/WinbondGamingKeyboardController (GPL-2.0-or-later).

use super::common::*;
use super::klm::{EXTRA, FN_ROW, FULL, Key, Klm, Layout, MAIN, NUMPAD, Op as KeyOp, SIXTY, TKL, k};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color};
use hidapi::HidApi;

const FAMILY: &str = "winbond";
const LEN: usize = 64;
const STATIC: u8 = 0;
const CUSTOM: u8 = 10;
const LOGO: u32 = kv(255, 0);

const fn kv(msg: u32, idx: u32) -> u32 {
    msg << 8 | idx | 1 << 16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub size: u8,
    pub layout: Layout,
    pub logo: bool,
    pub hator: bool,
}

/// SetNameVendorDescription: what the product string says about the board.
pub fn model_from_name(name: &str) -> Model {
    let mut m = Model {
        size: FULL,
        layout: Layout::IsoQwerty,
        logo: false,
        hator: false,
    };
    if name.contains("PCMK") {
        if name.contains("TKL") {
            m.size = TKL;
            m.logo = true;
        } else if name.contains("60") {
            m.size = SIXTY;
        }
    } else if name.contains("Rockfall") || name.contains("Skyfall") {
        m.hator = true;
        m.layout = Layout::AnsiQwerty;
        if name.contains("TKL") {
            m.size = TKL;
        }
    }
    m
}

/// ParseVersionString: the two letters before "CVRGB" name the layout.
pub fn layout_from_version(text: &str, current: Layout) -> Layout {
    match text.find("CVRGB") {
        Some(at) if at > 2 => match &text.as_bytes()[at - 2..at] {
            b"UK" => Layout::IsoQwerty,
            b"JP" => Layout::Jis,
            _ => Layout::AnsiQwerty,
        },
        _ => current,
    }
}

pub fn version_request() -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[0] = 1;
    b[1] = 0x0D;
    b
}

/// SetVersionLayout: asks for the version string and reads the layout from it.
pub fn read_layout(io: &mut dyn Io, current: Layout) -> Result<Layout, String> {
    io.write(&version_request())?;
    for _ in 0..10 {
        let mut reply = [0u8; LEN + 1];
        io.read(&mut reply[..LEN], 150)?;
        if reply[0] != 1 || reply[1] != 0x0D || reply[4] != 0 {
            continue;
        }
        let text: Vec<u8> = reply[2..]
            .iter()
            .copied()
            .skip_while(|&b| b <= b' ')
            .collect();
        let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
        if end > 0 {
            return Ok(layout_from_version(
                &String::from_utf8_lossy(&text[..end]),
                current,
            ));
        }
    }
    Ok(current)
}

/// InitLayouts: key values in the order the layout manager assigns them.
pub fn values(size: u8, hator: bool) -> Vec<u32> {
    let mut v = Vec::new();
    let has = |z: u8| size & z != 0;
    if has(FN_ROW) {
        v.push(kv(0, 0));
        v.extend((2..=5).map(|i| kv(0, i)));
        v.extend((7..=14).map(|i| kv(0, i)));
    }
    if has(EXTRA) {
        v.extend((15..=17).map(|i| kv(0, i)));
    }
    if has(MAIN) {
        v.extend((4..=16).map(|i| kv(1, i)));
        v.push(kv(2, 0));
    }
    if has(EXTRA) {
        v.extend((1..=3).map(|i| kv(2, i)));
    }
    if has(NUMPAD) {
        v.extend((4..=7).map(|i| kv(2, i)));
    }
    if has(MAIN) {
        v.extend((8..=17).map(|i| kv(2, i)));
        v.extend((0..=2).map(|i| kv(3, i)));
        v.push(if hator { kv(3, 4) } else { kv(3, 3) });
    }
    if has(EXTRA) {
        v.extend((5..=7).map(|i| kv(3, i)));
    }
    if has(NUMPAD) {
        v.extend((8..=11).map(|i| kv(3, i)));
    }
    if has(MAIN) {
        v.push(kv(3, 12));
        v.extend((14..=17).map(|i| kv(3, i)));
        v.extend((0..=8).map(|i| kv(4, i)));
    }
    if has(NUMPAD) {
        v.extend((12..=14).map(|i| kv(4, i)));
    }
    if has(MAIN) {
        v.push(kv(4, 16));
        v.push(kv(4, 17));
        v.extend((0..=9).map(|i| kv(5, i)));
        v.push(kv(5, 12));
    }
    if has(EXTRA) {
        v.push(kv(5, 14));
    }
    if has(NUMPAD) {
        v.push(kv(5, 16));
        v.push(kv(5, 17));
        v.extend((0..=1).map(|i| kv(6, i)));
    }
    if has(MAIN) {
        v.extend((2..=4).map(|i| kv(6, i)));
        v.push(kv(6, 8));
        v.extend((12..=15).map(|i| kv(6, i)));
    }
    if has(EXTRA) {
        v.push(kv(6, 17));
        v.push(kv(7, 0));
        v.push(kv(7, 1));
    }
    if has(NUMPAD) {
        v.push(kv(7, 3));
        v.push(kv(7, 5));
    }
    v
}

const MEDIA: [Key; 4] = [
    k(0, 17, kv(1, 0), "Media Previous", KeyOp::InsertShiftRight),
    k(0, 18, kv(1, 1), "Media Play/Pause", KeyOp::InsertShiftRight),
    k(0, 19, kv(1, 2), "Media Next", KeyOp::InsertShiftRight),
    k(0, 20, kv(1, 3), "Media Mute", KeyOp::InsertShiftRight),
];

fn keyboard(m: Model) -> Klm {
    let mut overlay: Vec<Key> = match m.layout {
        Layout::IsoQwerty => vec![
            k(3, 12, kv(4, 7), "#", KeyOp::SwapOnly),
            k(4, 1, kv(4, 17), "\\ (ISO)", KeyOp::SwapOnly),
        ],
        Layout::Jis => vec![k(1, 13, kv(1, 17), "¥", KeyOp::InsertShiftRight)],
        _ => vec![],
    };
    if m.size == FULL {
        overlay.extend(MEDIA);
    }
    Klm::new(m.layout, m.size, &values(m.size, m.hator), &overlay)
}

fn mode_report(
    logo: bool,
    mode: u8,
    colors: [[u8; 3]; 2],
    full: bool,
    dir: u8,
    speed: u8,
    brightness: u8,
) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[..9].copy_from_slice(&[1, 7, 0, 0, 0, 0x0E, mode, brightness, speed]);
    if logo {
        b[1] = 8;
        b[5] = 0x0D;
    }
    b[9..12].copy_from_slice(&colors[0]);
    b[12..15].copy_from_slice(&colors[1]);
    b[15] = dir;
    b[16] = full as u8;
    b
}

const NO_COLOURS: [[u8; 3]; 2] = [[0, 0, 255], [0, 0, 0]];
/// Custom's defaults: brightness 1, speed 2, direction left (0 for this effect).
const BRIGHTNESS: u8 = 1;
const SPEED: u8 = 2;

/// SetMode(Custom), plus the logo report when the board has one.
pub fn custom_mode(logo: bool) -> Vec<[u8; LEN]> {
    let mut out = vec![mode_report(
        false, CUSTOM, NO_COLOURS, true, 0, SPEED, BRIGHTNESS,
    )];
    if logo {
        out.push(mode_report(
            true, STATIC, NO_COLOURS, true, 0, 0, BRIGHTNESS,
        ));
    }
    out
}

fn scale(v: u8) -> u8 {
    (v as u32 * 0xC1 / 0xFF) as u8
}

/// SetLEDsData: eight 64-byte messages, then the logo as a static colour.
pub fn leds_data(values: &[u32], colors: &[Color], logo: bool) -> Vec<[u8; LEN]> {
    let mut msgs = [[0u8; LEN]; 8];
    for (i, m) in msgs.iter_mut().enumerate() {
        m[..6].copy_from_slice(&[1, 9, 0, 0, i as u8, if i == 7 { 0x12 } else { 0x36 }]);
    }
    let mut logo_color = Color(128, 128, 128);
    for (&val, &c) in values.iter().zip(colors) {
        if val == 0 {
            continue;
        }
        let msg = (val >> 8) & 255;
        if msg == 255 {
            logo_color = c;
            continue;
        }
        let o = (val & 255) as usize * 3 + 6;
        msgs[(msg & 7) as usize][o..o + 3].copy_from_slice(&[scale(c.0), scale(c.1), scale(c.2)]);
    }
    let mut out = msgs.to_vec();
    if logo {
        out.push(mode_report(
            true,
            STATIC,
            [rgb(logo_color), [0, 0, 0]],
            false,
            0,
            3,
            BRIGHTNESS,
        ));
    }
    out
}

struct Winbond {
    values: Vec<u32>,
    logo: bool,
}

impl Proto for Winbond {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        custom_mode(self.logo).iter().try_for_each(|p| io.write(p))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        leds_data(&self.values, colors, self.logo)
            .iter()
            .try_for_each(|p| io.write(p))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let mut model = model_from_name(&info.product);
    model.layout = read_layout(io.as_mut(), model.layout)?;
    let kb = keyboard(model);
    let mut values = kb.values();
    let mut dev = kb.device(info, FAMILY, d);
    if model.logo {
        values.push(LOGO);
        dev.leds.push([0.5, 0.02]);
    }
    let proto = Winbond {
        values,
        logo: model.logo,
    };
    Ok(Box::new(Driver::new(io, dev, FAMILY, proto)))
}

pub static DETECTORS: &[Detector] = &[det(
    "Winbond Gaming Keyboard",
    0x0416,
    0xB23C,
    ANY,
    0xFF1B,
    0x91,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn models() {
        let m = model_from_name("PCMK TKL");
        assert_eq!(
            (m.size, m.logo, m.layout == Layout::IsoQwerty),
            (TKL, true, true)
        );
        let m = model_from_name("Hator Rockfall 2");
        assert!(m.hator && m.size == FULL && m.layout == Layout::AnsiQwerty);
        assert!(layout_from_version("V1.2 UKCVRGB", Layout::AnsiQwerty) == Layout::IsoQwerty);
        assert!(layout_from_version("V1.2 JPCVRGB", Layout::IsoQwerty) == Layout::Jis);
        assert!(layout_from_version("V1.2 USCVRGB", Layout::IsoQwerty) == Layout::AnsiQwerty);
        assert!(layout_from_version("V1.2", Layout::IsoQwerty) == Layout::IsoQwerty);
    }

    // SetVersionLayout skips replies that are not version replies
    #[test]
    fn version_read() {
        let mut good = vec![1, 0x0D, b' ', b' ', 0];
        good.extend(b"V1 UKCVRGB");
        let mut rec = Rec::with(vec![vec![1, 0x0E], good]);
        assert!(read_layout(&mut rec, Layout::AnsiQwerty).unwrap() == Layout::IsoQwerty);
        assert_eq!(
            rec.take(),
            vec![w(version_request().to_vec()), Op::Read, Op::Read]
        );
    }

    #[test]
    fn value_counts_match_layouts() {
        assert_eq!(values(FULL, false).len(), 106);
        assert_eq!(values(TKL, false).len(), 89);
        let kb = keyboard(model_from_name("x"));
        assert_eq!(kb.keys.len(), 105 + 4);
    }

    // SetMode(Custom) with the logo, then SetLEDsData
    #[test]
    fn custom_then_leds() {
        let model = model_from_name("PCMK TKL");
        let kb = keyboard(model);
        let mut values = kb.values();
        values.push(LOGO);
        let n = values.len();
        let mut a = vec![Color::BLACK; n];
        a[0] = Color(255, 128, 0); // Escape, KV(0,0)
        a[n - 1] = Color(9, 8, 7);
        let [first, _, _] = pushes(Winbond { values, logo: true }, n, vec![], &a, &[]);
        assert_eq!(first.len(), 2 + 8 + 1);
        assert_eq!(
            first[0],
            w(pad(
                &[1, 7, 0, 0, 0, 0x0E, 10, 1, 2, 0, 0, 255, 0, 0, 0, 0, 1],
                64
            ))
        );
        assert_eq!(
            first[1],
            w(pad(
                &[1, 8, 0, 0, 0, 0x0D, 0, 1, 0, 0, 0, 255, 0, 0, 0, 0, 1],
                64
            ))
        );
        assert_eq!(first[2], w(pad(&[1, 9, 0, 0, 0, 0x36, 0xC1, 0x60, 0], 64)));
        assert_eq!(
            first[10],
            w(pad(
                &[1, 8, 0, 0, 0, 0x0D, 0, 1, 3, 9, 8, 7, 0, 0, 0, 0, 0],
                64
            ))
        );
    }
}
