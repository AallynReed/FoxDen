//! USB HID keyboard usages (page 0x07) ↔ PC set-1 scancodes, the form Windows' SendInput and
//! low-level hooks use. Extended keys carry the E0 prefix flag.

/// `(usage, scancode, extended, name)`
const TABLE: &[(u16, u16, bool, &str)] = &[
    (0x04, 0x1E, false, "A"), (0x05, 0x30, false, "B"), (0x06, 0x2E, false, "C"), (0x07, 0x20, false, "D"),
    (0x08, 0x12, false, "E"), (0x09, 0x21, false, "F"), (0x0A, 0x22, false, "G"), (0x0B, 0x23, false, "H"),
    (0x0C, 0x17, false, "I"), (0x0D, 0x24, false, "J"), (0x0E, 0x25, false, "K"), (0x0F, 0x26, false, "L"),
    (0x10, 0x32, false, "M"), (0x11, 0x31, false, "N"), (0x12, 0x18, false, "O"), (0x13, 0x19, false, "P"),
    (0x14, 0x10, false, "Q"), (0x15, 0x13, false, "R"), (0x16, 0x1F, false, "S"), (0x17, 0x14, false, "T"),
    (0x18, 0x16, false, "U"), (0x19, 0x2F, false, "V"), (0x1A, 0x11, false, "W"), (0x1B, 0x2D, false, "X"),
    (0x1C, 0x15, false, "Y"), (0x1D, 0x2C, false, "Z"),
    (0x1E, 0x02, false, "1"), (0x1F, 0x03, false, "2"), (0x20, 0x04, false, "3"), (0x21, 0x05, false, "4"),
    (0x22, 0x06, false, "5"), (0x23, 0x07, false, "6"), (0x24, 0x08, false, "7"), (0x25, 0x09, false, "8"),
    (0x26, 0x0A, false, "9"), (0x27, 0x0B, false, "0"),
    (0x28, 0x1C, false, "Enter"), (0x29, 0x01, false, "Esc"), (0x2A, 0x0E, false, "Backspace"),
    (0x2B, 0x0F, false, "Tab"), (0x2C, 0x39, false, "Space"), (0x2D, 0x0C, false, "-"), (0x2E, 0x0D, false, "="),
    (0x2F, 0x1A, false, "["), (0x30, 0x1B, false, "]"), (0x31, 0x2B, false, "\\"), (0x33, 0x27, false, ";"),
    (0x34, 0x28, false, "'"), (0x35, 0x29, false, "`"), (0x36, 0x33, false, ","), (0x37, 0x34, false, "."),
    (0x38, 0x35, false, "/"), (0x39, 0x3A, false, "Caps Lock"),
    (0x3A, 0x3B, false, "F1"), (0x3B, 0x3C, false, "F2"), (0x3C, 0x3D, false, "F3"), (0x3D, 0x3E, false, "F4"),
    (0x3E, 0x3F, false, "F5"), (0x3F, 0x40, false, "F6"), (0x40, 0x41, false, "F7"), (0x41, 0x42, false, "F8"),
    (0x42, 0x43, false, "F9"), (0x43, 0x44, false, "F10"), (0x44, 0x57, false, "F11"), (0x45, 0x58, false, "F12"),
    (0x46, 0x37, true, "Print Screen"), (0x47, 0x46, false, "Scroll Lock"),
    (0x49, 0x52, true, "Insert"), (0x4A, 0x47, true, "Home"), (0x4B, 0x49, true, "Page Up"),
    (0x4C, 0x53, true, "Delete"), (0x4D, 0x4F, true, "End"), (0x4E, 0x51, true, "Page Down"),
    (0x4F, 0x4D, true, "Right"), (0x50, 0x4B, true, "Left"), (0x51, 0x50, true, "Down"), (0x52, 0x48, true, "Up"),
    (0x53, 0x45, false, "Num Lock"), (0x54, 0x35, true, "Num /"), (0x55, 0x37, false, "Num *"),
    (0x56, 0x4A, false, "Num -"), (0x57, 0x4E, false, "Num +"), (0x58, 0x1C, true, "Num Enter"),
    (0x59, 0x4F, false, "Num 1"), (0x5A, 0x50, false, "Num 2"), (0x5B, 0x51, false, "Num 3"),
    (0x5C, 0x4B, false, "Num 4"), (0x5D, 0x4C, false, "Num 5"), (0x5E, 0x4D, false, "Num 6"),
    (0x5F, 0x47, false, "Num 7"), (0x60, 0x48, false, "Num 8"), (0x61, 0x49, false, "Num 9"),
    (0x62, 0x52, false, "Num 0"), (0x63, 0x53, false, "Num ."), (0x64, 0x56, false, "\\ (ISO)"),
    (0x65, 0x5D, true, "Menu"),
    (0x68, 0x64, false, "F13"), (0x69, 0x65, false, "F14"), (0x6A, 0x66, false, "F15"), (0x6B, 0x67, false, "F16"),
    (0x6C, 0x68, false, "F17"), (0x6D, 0x69, false, "F18"), (0x6E, 0x6A, false, "F19"), (0x6F, 0x6B, false, "F20"),
    (0x70, 0x6C, false, "F21"), (0x71, 0x6D, false, "F22"), (0x72, 0x6E, false, "F23"), (0x73, 0x76, false, "F24"),
    (0xE0, 0x1D, false, "Ctrl"), (0xE1, 0x2A, false, "Shift"), (0xE2, 0x38, false, "Alt"), (0xE3, 0x5B, true, "Win"),
    (0xE4, 0x1D, true, "Right Ctrl"), (0xE5, 0x36, false, "Right Shift"), (0xE6, 0x38, true, "Right Alt"),
    (0xE7, 0x5C, true, "Right Win"),
];

/// Pause has no single scancode; it's sent as a virtual key.
pub const PAUSE_USAGE: u16 = 0x48;
pub const VK_PAUSE: u16 = 0x13;

pub fn scancode(usage: u16) -> Option<(u16, bool)> {
    TABLE.iter().find(|r| r.0 == usage).map(|r| (r.1, r.2))
}

pub fn usage_from_scancode(scan: u16, extended: bool) -> Option<u16> {
    TABLE.iter().find(|r| r.1 == scan && r.2 == extended).map(|r| r.0)
}

pub fn name(usage: u16) -> &'static str {
    if usage == PAUSE_USAGE {
        return "Pause";
    }
    TABLE.iter().find(|r| r.0 == usage).map(|r| r.3).unwrap_or("Key")
}

const ALIASES: &[(&str, u16)] = &[
    ("Control", 0xE0), ("LCtrl", 0xE0), ("RCtrl", 0xE4), ("LShift", 0xE1), ("RShift", 0xE5), ("LAlt", 0xE2),
    ("RAlt", 0xE6), ("AltGr", 0xE6), ("Windows", 0xE3), ("Meta", 0xE3), ("Escape", 0x29), ("Return", 0x28),
    ("Del", 0x4C), ("Ins", 0x49), ("PgUp", 0x4B), ("PgDn", 0x4E), ("PrtSc", 0x46), ("Caps", 0x39),
    ("ArrowUp", 0x52), ("ArrowDown", 0x51), ("ArrowLeft", 0x50), ("ArrowRight", 0x4F), ("Minus", 0x2D),
    ("Equals", 0x2E), ("Comma", 0x36), ("Period", 0x37), ("Slash", 0x38), ("Semicolon", 0x33),
    ("Quote", 0x34), ("Backtick", 0x35), ("Backslash", 0x31),
];

/// A usage from its name ("F13", "Page Up", "Esc"), ignoring case and spaces.
pub fn by_name(name: &str) -> Option<u16> {
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    let want = squash(name);
    if want.is_empty() {
        return None;
    }
    if want == "pause" {
        return Some(PAUSE_USAGE);
    }
    TABLE
        .iter()
        .find(|r| squash(r.3) == want)
        .map(|r| r.0)
        .or_else(|| ALIASES.iter().find(|a| squash(a.0) == want).map(|a| a.1))
}

/// Every key FoxDen can send, by usage and name.
pub fn all() -> impl Iterator<Item = (u16, &'static str)> {
    TABLE.iter().map(|r| (r.0, r.3))
}

pub fn is_modifier(usage: u16) -> bool {
    (0xE0..=0xE7).contains(&usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_keys_are_flagged() {
        for u in [0x46u16, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x54, 0x58, 0xE3, 0xE4, 0xE6, 0xE7] {
            assert!(scancode(u).unwrap().1, "usage {u:#x} needs E0");
        }
        assert_eq!(scancode(0x04), Some((0x1E, false)));
        assert_eq!(scancode(0xE4), Some((0x1D, true)));
        assert_eq!(usage_from_scancode(0x1D, true), Some(0xE4));
        assert_eq!(usage_from_scancode(0x1D, false), Some(0xE0));
    }

    #[test]
    fn names_resolve() {
        assert_eq!(by_name("page up"), Some(0x4B));
        assert_eq!(by_name("ESCAPE"), Some(0x29));
        assert_eq!(by_name("f24"), Some(0x73));
        assert_eq!(by_name("Num Enter"), Some(0x58));
        assert_eq!(by_name("Pause"), Some(PAUSE_USAGE));
        assert_eq!(by_name("nope"), None);
        for (u, n) in all() {
            assert_eq!(by_name(n), Some(u), "{n}");
        }
    }

    #[test]
    fn round_trip() {
        for r in TABLE {
            assert_eq!(usage_from_scancode(r.1, r.2), Some(r.0), "{}", r.3);
        }
    }
}
