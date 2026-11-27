//! Steps → the low-level inputs a player sends. Pure, so it can be tested without typing anything.

use crate::keys;
use crate::model::{MediaKey, Step};

pub const WHEEL_DELTA: i32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Raw {
    Scan { code: u16, extended: bool, up: bool },
    Vk { vk: u16, up: bool },
    Unicode { unit: u16, up: bool },
    Button { button: u8, up: bool },
    Wheel { delta: i32, horizontal: bool },
    Sleep { ms: u32 },
}

pub fn media_vk(k: MediaKey) -> u16 {
    match k {
        MediaKey::Mute => 0xAD,
        MediaKey::VolumeDown => 0xAE,
        MediaKey::VolumeUp => 0xAF,
        MediaKey::Next => 0xB0,
        MediaKey::Previous => 0xB1,
        MediaKey::Stop => 0xB2,
        MediaKey::PlayPause => 0xB3,
    }
}

fn step(s: &Step, out: &mut Vec<Raw>) {
    match s {
        Step::Key { usage, down } => {
            if *usage == keys::PAUSE_USAGE {
                out.push(Raw::Vk { vk: keys::VK_PAUSE, up: !down });
            } else if let Some((code, extended)) = keys::scancode(*usage) {
                out.push(Raw::Scan { code, extended, up: !down });
            }
        }
        Step::Button { button, down } => out.push(Raw::Button { button: *button, up: !down }),
        Step::Wheel { horizontal, ticks } => out.push(Raw::Wheel { delta: ticks * WHEEL_DELTA, horizontal: *horizontal }),
        Step::Delay { ms } => out.push(Raw::Sleep { ms: *ms }),
        Step::Text { text } => {
            for unit in text.encode_utf16() {
                out.push(Raw::Unicode { unit, up: false });
                out.push(Raw::Unicode { unit, up: true });
            }
        }
        Step::Media { key } => {
            let vk = media_vk(*key);
            out.push(Raw::Vk { vk, up: false });
            out.push(Raw::Vk { vk, up: true });
        }
    }
}

/// Expands steps; with a default delay, a pause goes between steps that don't set their own.
pub fn expand(steps: &[Step], default_delay_ms: Option<u32>) -> Vec<Raw> {
    let mut out = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        step(s, &mut out);
        if let Some(d) = default_delay_ms {
            let next_is_delay = matches!(steps.get(i + 1), Some(Step::Delay { .. }));
            if !matches!(s, Step::Delay { .. }) && !next_is_delay && i + 1 < steps.len() {
                out.push(Raw::Sleep { ms: d });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Macro;

    #[test]
    fn chord_becomes_scancodes() {
        let m = Macro::chord("copy", &[0xE0], 0x06);
        let r = expand(&m.press, None);
        assert_eq!(
            r,
            vec![
                Raw::Scan { code: 0x1D, extended: false, up: false },
                Raw::Scan { code: 0x2E, extended: false, up: false },
                Raw::Scan { code: 0x2E, extended: false, up: true },
                Raw::Scan { code: 0x1D, extended: false, up: true },
            ]
        );
        let up = expand(&[Step::Key { usage: 0x52, down: true }], None);
        assert_eq!(up, vec![Raw::Scan { code: 0x48, extended: true, up: false }]);
    }

    #[test]
    fn text_uses_surrogate_pairs() {
        let r = expand(&[Step::Text { text: "a😀".into() }], None);
        assert_eq!(r.len(), 6);
        assert_eq!(r[2], Raw::Unicode { unit: 0xD83D, up: false });
        assert_eq!(r[4], Raw::Unicode { unit: 0xDE00, up: false });
    }

    #[test]
    fn default_delay_between_steps() {
        let s = [Step::Key { usage: 4, down: true }, Step::Key { usage: 4, down: false }, Step::Delay { ms: 7 }, Step::Wheel { horizontal: false, ticks: -2 }];
        let r = expand(&s, Some(20));
        assert_eq!(r[1], Raw::Sleep { ms: 20 });
        assert_eq!(r[3], Raw::Sleep { ms: 7 });
        assert_eq!(r[4], Raw::Wheel { delta: -240, horizontal: false });
        assert_eq!(r.len(), 5);
        assert_eq!(expand(&[Step::Media { key: MediaKey::PlayPause }], None)[0], Raw::Vk { vk: 0xB3, up: false });
    }
}
