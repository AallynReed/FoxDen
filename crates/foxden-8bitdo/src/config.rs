//! A pad's config block: three profiles (two on the HitBox 2) of every setting the vendor app
//! edits, read and written as one block or one section at a time.

use serde::{Deserialize, Serialize};

use crate::model::{
    COMBO_LEN, Layout, LightLayout, MACRO_LEN, NAME_LEN, RECORDED_LEN, RGB_RAW_LEN,
};
use crate::proto::crc16;

/// The marker the vendor app writes into a section it has set.
pub const SET: u32 = 0x2020_0911;
/// The marker of a profile the app turned off.
pub const UNSET: u32 = 0x2019_0000;

/// Output mode the pad starts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Switch,
    DInput,
    Mac,
    XInput,
    Extended,
    Other(u16),
}

impl Mode {
    pub fn code(self) -> u16 {
        match self {
            Mode::Switch => 0,
            Mode::DInput => 1,
            Mode::Mac => 2,
            Mode::XInput => 3,
            Mode::Extended => 4,
            Mode::Other(n) => n,
        }
    }

    pub fn from_code(n: u16) -> Mode {
        [
            Mode::Switch,
            Mode::DInput,
            Mode::Mac,
            Mode::XInput,
            Mode::Extended,
        ]
        .into_iter()
        .find(|m| m.code() == n)
        .unwrap_or(Mode::Other(n))
    }
}

/// Bits of a profile's `features` word, named as the vendor app shows them.
pub mod feature {
    pub const INVERT_LEFT_X: u32 = 1 << 0;
    pub const INVERT_LEFT_Y: u32 = 1 << 1;
    pub const INVERT_RIGHT_X: u32 = 1 << 2;
    pub const INVERT_RIGHT_Y: u32 = 1 << 3;
    pub const SWAP_STICKS: u32 = 1 << 4;
    pub const SWAP_TRIGGERS: u32 = 1 << 7;
    pub const LEFT_STICK_AS_DPAD: u32 = 1 << 8;
    pub const TRIGGERS_AS_BUTTONS: u32 = 1 << 9;
    pub const RIGHT_STICK_AS_TRIGGERS: u32 = 1 << 10;
    pub const MOTION_BOOST: u32 = 1 << 11;
    pub const DEADZONE_OFF: u32 = 1 << 12;
    pub const FOUR_WAY: u32 = 1 << 13;
    pub const SLEEP: u32 = 1 << 14;
    pub const VIBRATION_OFF: u32 = 1 << 15;
    pub const STICK_UP: u32 = 1 << 16;
    pub const STICK_FRONT: u32 = 1 << 17;
    pub const STICK_BACK: u32 = 1 << 18;
}

/// Output buttons a physical button can send; a mapping is their OR.
pub mod button {
    pub const START: u32 = 1;
    pub const L3: u32 = 2;
    pub const R3: u32 = 4;
    pub const SELECT: u32 = 8;
    pub const X: u32 = 0x10;
    pub const Y: u32 = 0x20;
    pub const RIGHT: u32 = 0x40;
    pub const LEFT: u32 = 0x80;
    pub const DOWN: u32 = 0x100;
    pub const UP: u32 = 0x200;
    pub const L1: u32 = 0x400;
    pub const R1: u32 = 0x800;
    pub const B: u32 = 0x1000;
    pub const A: u32 = 0x2000;
    pub const L2: u32 = 0x4000;
    pub const HOME: u32 = 0x8000;
    pub const MENU: u32 = 0x1_0000;
    pub const R2: u32 = 0x2_0000;
    pub const PAIR: u32 = 0x4_0000;
    pub const SCREENSHOT: u32 = 0x40_0000;
    pub const TURBO: u32 = 0x80_0000;
    pub const TURBO_AUTO: u32 = 0x100_0000;
    /// Right stick directions carry this bit with UP/DOWN/LEFT/RIGHT's low nibble.
    pub const RIGHT_STICK: u32 = 0x0800_0000;
    pub const NONE: u32 = 0;

    /// The order of a profile's 22 key slots (the last four are the extra buttons).
    pub const ORDER: [&str; 22] = [
        "A", "B", "X", "Y", "L1", "R1", "L2", "R2", "L3", "R3", "Select", "Start", "Star", "Home",
        "Up", "Down", "Left", "Right", "P1", "P2", "P3", "P4",
    ];

    /// What each slot sends when nothing is remapped, in Switch mode.
    pub const SWITCH: [u32; 22] = [
        A, B, X, Y, L1, R1, L2, R2, L3, R3, SELECT, START, SCREENSHOT, HOME, UP, DOWN, LEFT, RIGHT,
        NONE, NONE, NONE, NONE,
    ];
    /// In every other mode A/B and X/Y trade places.
    pub const XINPUT: [u32; 22] = [
        B, A, Y, X, L1, R1, L2, R2, L3, R3, SELECT, START, MENU, HOME, UP, DOWN, LEFT, RIGHT, NONE,
        NONE, NONE, NONE,
    ];
}

/// A pair of 0..=100 bytes per side: where travel starts counting and where it reads as full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: u8,
    pub end: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub left: Range,
    pub right: Range,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rumble {
    /// Motor strength, 0.0..=1.0 (the app offers 0, 0.2, 0.4 ... 1.0).
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Motion {
    /// Button mask that turns motion aiming on.
    pub keys: u32,
    pub trigger_mode: u8,
    pub sensitivity: u8,
    pub dead_compensate: u8,
    pub mapping: u8,
}

/// Colours are 0x00RRGGBB.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lights {
    pub dynamic: u32,
    pub dynamic_back: u32,
    pub wheel: u32,
    pub wheel_back: u32,
    pub wheel_speed: u8,
    /// One colour per LED: 12 round the left stick, then 12 round the right.
    pub leds: Vec<u32>,
}

/// One profile's settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub enabled: bool,
    pub name: String,
    pub rumble: Rumble,
    pub sticks: Pair,
    pub triggers: Pair,
    pub features: u32,
    pub keys: Vec<u32>,
    pub xinput_rumble: Option<Pair>,
    pub motion: Option<Motion>,
    pub lights: Option<Lights>,
}

/// A byte range of the block, for writing one section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub at: usize,
    pub len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub layout: Layout,
    pub bytes: Vec<u8>,
}

fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn wr32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn rd16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn range(b: &[u8], at: usize) -> Pair {
    Pair {
        left: Range {
            start: b[at],
            end: b[at + 1],
        },
        right: Range {
            start: b[at + 2],
            end: b[at + 3],
        },
    }
}

fn put_range(b: &mut [u8], at: usize, p: Pair) {
    b[at..at + 4].copy_from_slice(&[p.left.start, p.left.end, p.right.start, p.right.end]);
}

/// Names are UTF-16 big-endian, ending at the first zero pair (an erased name is all 0xFF).
fn name(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>().0.iter()
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0 && u != 0xFFFF)
        .collect();
    String::from_utf16_lossy(&units)
}

fn put_name(b: &mut [u8], s: &str) {
    b.fill(0);
    for (i, u) in s.encode_utf16().take(NAME_LEN / 2 - 1).enumerate() {
        b[i * 2..i * 2 + 2].copy_from_slice(&u.to_be_bytes());
    }
}

impl Config {
    pub fn new(layout: Layout, bytes: Vec<u8>) -> Result<Config, String> {
        if bytes.len() != layout.size {
            return Err(format!("{} bytes, expected {}", bytes.len(), layout.size));
        }
        Ok(Config { layout, bytes })
    }

    fn check(&self, slot: usize) -> Result<(), String> {
        (slot < self.layout.slots)
            .then_some(())
            .ok_or_else(|| format!("no profile {slot}"))
    }

    pub fn mode(&self) -> Mode {
        Mode::from_code(rd16(&self.bytes, self.layout.mode))
    }

    /// The profile in use, 0-based.
    pub fn current(&self) -> u16 {
        rd16(&self.bytes, self.layout.slot)
    }

    pub fn crc(&self) -> u32 {
        rd32(&self.bytes, self.layout.crc)
    }

    /// The vendor app sums the block as it stands, old CRC included, and stores the result.
    pub fn seal(&mut self) {
        let c = crc16(&self.bytes) as u32;
        wr32(&mut self.bytes, self.layout.crc, c);
    }

    fn key_len(&self) -> usize {
        4 + 4 * self.layout.key_count
    }

    pub fn profile(&self, slot: usize) -> Result<Profile, String> {
        self.check(slot)?;
        let (l, b) = (&self.layout, &self.bytes);
        let n = l.names + slot * NAME_LEN;
        let r = l.rumble + slot * 12;
        let k = l.keys + slot * self.key_len() + 4;
        Ok(Profile {
            enabled: rd32(b, slot * 4) == SET,
            name: name(&b[n..n + NAME_LEN]),
            rumble: Rumble {
                left: f32::from_le_bytes(b[r + 4..r + 8].try_into().unwrap()),
                right: f32::from_le_bytes(b[r + 8..r + 12].try_into().unwrap()),
            },
            sticks: range(b, l.sticks + slot * 8 + 4),
            triggers: range(b, l.triggers + slot * 8 + 4),
            features: rd32(b, l.features + slot * 8 + 4),
            keys: (0..l.key_count).map(|i| rd32(b, k + i * 4)).collect(),
            xinput_rumble: l.xinput_rumble.map(|at| range(b, at + slot * 8 + 4)),
            motion: l.motion.map(|at| {
                let at = at + slot * 12;
                Motion {
                    keys: rd32(b, at + 4),
                    trigger_mode: b[at + 8],
                    sensitivity: b[at + 9],
                    dead_compensate: b[at + 10],
                    mapping: b[at + 11],
                }
            }),
            lights: l.light.map(|li| self.lights(li, slot)),
        })
    }

    fn lights(&self, li: LightLayout, slot: usize) -> Lights {
        let b = &self.bytes;
        let d = li.dynamic + slot * 12;
        let w = li.wheel + slot * 16;
        let s = li.single + slot * (4 + 4 * li.leds) + 4;
        Lights {
            dynamic: rd32(b, d + 4),
            dynamic_back: rd32(b, d + 8),
            wheel: rd32(b, w + 4),
            wheel_back: rd32(b, w + 8),
            wheel_speed: b[w + 12],
            leds: (0..li.leds).map(|i| rd32(b, s + i * 4)).collect(),
        }
    }

    /// Writes `p` into profile `slot`, marks every section it touched as set, and reseals the
    /// block. Returns the byte ranges that changed, for a partial write.
    pub fn set_profile(&mut self, slot: usize, p: &Profile) -> Result<Vec<Span>, String> {
        self.check(slot)?;
        let l = self.layout;
        if p.keys.len() != l.key_count {
            return Err(format!("{} keys, expected {}", p.keys.len(), l.key_count));
        }
        if let (Some(li), Some(x)) = (l.light, &p.lights)
            && x.leds.len() != li.leds
        {
            return Err(format!("{} LEDs, expected {}", x.leds.len(), li.leds));
        }
        let before = self.bytes.clone();
        let key_len = self.key_len();
        let b = &mut self.bytes;
        wr32(b, slot * 4, if p.enabled { SET } else { UNSET });
        let n = l.names + slot * NAME_LEN;
        put_name(&mut b[n..n + NAME_LEN], &p.name);
        let r = l.rumble + slot * 12;
        wr32(b, r, SET);
        b[r + 4..r + 8].copy_from_slice(&p.rumble.left.clamp(0.0, 1.0).to_le_bytes());
        b[r + 8..r + 12].copy_from_slice(&p.rumble.right.clamp(0.0, 1.0).to_le_bytes());
        for (at, pair) in [(l.sticks, p.sticks), (l.triggers, p.triggers)] {
            wr32(b, at + slot * 8, SET);
            put_range(b, at + slot * 8 + 4, pair);
        }
        wr32(b, l.features + slot * 8, SET);
        wr32(b, l.features + slot * 8 + 4, p.features);
        let k = l.keys + slot * key_len;
        wr32(b, k, SET);
        for (i, v) in p.keys.iter().enumerate() {
            wr32(b, k + 4 + i * 4, *v);
        }
        if let (Some(at), Some(pair)) = (l.xinput_rumble, p.xinput_rumble) {
            wr32(b, at + slot * 8, SET);
            put_range(b, at + slot * 8 + 4, pair);
        }
        if let (Some(at), Some(m)) = (l.motion, p.motion) {
            let at = at + slot * 12;
            wr32(b, at, SET);
            wr32(b, at + 4, m.keys);
            b[at + 8..at + 12].copy_from_slice(&[
                m.trigger_mode,
                m.sensitivity,
                m.dead_compensate,
                m.mapping,
            ]);
        }
        if let (Some(li), Some(lights)) = (l.light, &p.lights) {
            self.set_lights(li, slot, lights);
        }
        self.seal();
        Ok(diff(&before, &self.bytes))
    }

    fn set_lights(&mut self, li: LightLayout, slot: usize, x: &Lights) {
        let b = &mut self.bytes;
        let d = li.dynamic + slot * 12;
        wr32(b, d, SET);
        wr32(b, d + 4, x.dynamic & 0xFF_FFFF);
        wr32(b, d + 8, x.dynamic_back & 0xFF_FFFF);
        let w = li.wheel + slot * 16;
        wr32(b, w, SET);
        wr32(b, w + 4, x.wheel & 0xFF_FFFF);
        wr32(b, w + 8, x.wheel_back & 0xFF_FFFF);
        b[w + 12] = x.wheel_speed;
        self.put_leds(li, slot, &x.leds);
    }

    fn put_leds(&mut self, li: LightLayout, slot: usize, leds: &[u32]) {
        let s = li.single + slot * (4 + 4 * li.leds);
        wr32(&mut self.bytes, s, SET);
        for (i, c) in leds.iter().take(li.leds).enumerate() {
            wr32(&mut self.bytes, s + 4 + i * 4, c & 0xFF_FFFF);
        }
    }

    /// Only the custom LED colours of `slot`: the span the vendor app writes for them.
    pub fn set_leds(&mut self, slot: usize, leds: &[u32]) -> Result<Span, String> {
        self.check(slot)?;
        let li = self.layout.light.ok_or("this pad has no light ring")?;
        self.put_leds(li, slot, leds);
        self.seal();
        Ok(Span {
            at: li.single + slot * (4 + 4 * li.leds),
            len: 4 + 4 * li.leds,
        })
    }

    pub fn set_mode(&mut self, m: Mode) {
        let at = self.layout.mode;
        self.bytes[at..at + 2].copy_from_slice(&m.code().to_le_bytes());
        self.seal();
    }

    /// Raw records the protocol layer doesn't model yet: macros, HitBox 2 lighting and combos.
    pub fn raw(&self, what: Raw, slot: usize) -> Option<&[u8]> {
        let (at, len) = self.raw_span(what)?;
        (slot < self.layout.slots).then(|| &self.bytes[at + slot * len..at + (slot + 1) * len])
    }

    pub fn set_raw(&mut self, what: Raw, slot: usize, data: &[u8]) -> Result<Span, String> {
        self.check(slot)?;
        let (at, len) = self.raw_span(what).ok_or("this pad has no such record")?;
        if data.len() != len {
            return Err(format!("{} bytes, expected {len}", data.len()));
        }
        let at = at + slot * len;
        self.bytes[at..at + len].copy_from_slice(data);
        self.seal();
        Ok(Span { at, len })
    }

    fn raw_span(&self, what: Raw) -> Option<(usize, usize)> {
        let l = &self.layout;
        match what {
            Raw::Macros => l.macros.map(|a| (a, MACRO_LEN)),
            Raw::Recorded => l.recorded.map(|a| (a, RECORDED_LEN)),
            Raw::Rgb => l.rgb_raw.map(|a| (a, RGB_RAW_LEN)),
            Raw::Combos => l.combos.map(|a| (a, COMBO_LEN)),
        }
    }

    /// The CRC field, for sending after the spans that changed.
    pub fn crc_span(&self) -> Span {
        Span {
            at: self.layout.crc,
            len: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    Macros,
    Recorded,
    Rgb,
    Combos,
}

/// Runs of changed bytes, joined when fewer than 8 unchanged bytes lie between them.
pub fn diff(a: &[u8], b: &[u8]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for i in (0..a.len().min(b.len())).filter(|&i| a[i] != b[i]) {
        match out.last_mut() {
            Some(s) if i - (s.at + s.len) < 8 => s.len = i + 1 - s.at,
            _ => out.push(Span { at: i, len: 1 }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HITBOX2, PRO3, ULTIMATE2};

    fn blank(l: Layout) -> Config {
        Config::new(l, vec![0; l.size]).unwrap()
    }

    #[test]
    fn ultimate2_profile_round_trip() {
        let mut c = blank(ULTIMATE2);
        let p = Profile {
            enabled: true,
            name: "Racing".into(),
            rumble: Rumble {
                left: 0.6,
                right: 1.0,
            },
            sticks: Pair {
                left: Range { start: 5, end: 95 },
                right: Range { start: 8, end: 100 },
            },
            triggers: Pair::default(),
            features: feature::SWAP_STICKS | feature::INVERT_RIGHT_Y,
            keys: button::XINPUT.to_vec(),
            xinput_rumble: Some(Pair::default()),
            motion: Some(Motion {
                keys: button::L2,
                sensitivity: 7,
                ..Default::default()
            }),
            lights: Some(Lights {
                dynamic: 0xFF0000,
                leds: (0..24).map(|i| i * 0x0A0B0C).collect(),
                wheel_speed: 7,
                ..Default::default()
            }),
        };
        c.set_profile(1, &p).unwrap();
        assert_eq!(c.profile(1).unwrap(), p);
        // file_name[1] starts at 20 + 32, big-endian UTF-16.
        assert_eq!(&c.bytes[52..56], &[0, b'R', 0, b'a']);
        // sticks[1].range at 152 + 8 + 4.
        assert_eq!(&c.bytes[164..168], &[5, 95, 8, 100]);
        // single[1].color[0] at 1292 + 100 + 4.
        assert_eq!(rd32(&c.bytes, 1392), SET);
        assert_eq!(rd32(&c.bytes, 1396 + 4), 0x0A0B0C);
        assert!(c.profile(0).unwrap().name.is_empty());
    }

    #[test]
    fn seal_sums_the_block_with_its_old_crc() {
        let mut c = blank(PRO3);
        c.bytes[100] = 1;
        let want = crc16(&c.bytes);
        c.seal();
        assert_eq!(c.crc(), want as u32);
        let again = crc16(&c.bytes);
        c.seal();
        assert_eq!(c.crc(), again as u32);
    }

    #[test]
    fn leds_span_matches_the_vendor_write() {
        let mut c = blank(ULTIMATE2);
        let s = c.set_leds(2, &[0x123456; 24]).unwrap();
        // writeUltimate2single: 100 bytes at 0x50C + slot * 100.
        assert_eq!(
            s,
            Span {
                at: 0x50C + 200,
                len: 100
            }
        );
        assert!(c.set_leds(3, &[]).is_err());
        assert!(blank(PRO3).set_leds(0, &[0; 24]).is_err());
    }

    #[test]
    fn hitbox2_has_two_profiles_and_raw_lighting() {
        let mut c = blank(HITBOX2);
        assert!(c.profile(2).is_err());
        let s = c.set_raw(Raw::Rgb, 1, &[9; RGB_RAW_LEN]).unwrap();
        assert_eq!(s.at, 1584 + RGB_RAW_LEN);
        assert_eq!(c.raw(Raw::Rgb, 1).unwrap()[0], 9);
        assert!(c.raw(Raw::Macros, 0).is_some());
    }

    #[test]
    fn names_stop_at_zero_or_erased() {
        let mut b = [0xFFu8; 32];
        assert_eq!(name(&b), "");
        put_name(&mut b, "Ünï");
        assert_eq!(name(&b), "Ünï");
    }

    #[test]
    fn diff_joins_close_runs() {
        let a = [0u8; 40];
        let mut b = a;
        b[3] = 1;
        b[6] = 1;
        b[30] = 1;
        assert_eq!(
            diff(&a, &b),
            vec![Span { at: 3, len: 4 }, Span { at: 30, len: 1 }]
        );
    }

    #[test]
    fn modes() {
        let mut c = blank(PRO3);
        c.set_mode(Mode::XInput);
        assert_eq!(c.mode(), Mode::XInput);
        assert_eq!(Mode::from_code(9), Mode::Other(9));
    }
}
