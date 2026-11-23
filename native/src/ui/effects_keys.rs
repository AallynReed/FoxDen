//! Key names for the keyboard boards, and key text to Windows virtual-key codes.

const fn span<const N: usize>(from: u32) -> [u32; N] {
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        out[i] = from + i as u32;
        i += 1;
    }
    out
}

const F_KEYS: [u32; 12] = span(0x70);
const NUMBERS: [u32; 10] = span(0x30);
const LETTERS: [u32; 26] = span(0x41);
const ARROWS: [u32; 4] = span(0x25);
const PAD: [u32; 16] = [0x90, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6d, 0x6e, 0x6f];

pub const KEY_GROUPS: [Option<&[u32]>; 9] = [
    Some(&F_KEYS),
    Some(&NUMBERS),
    Some(&LETTERS),
    Some(&[0x57, 0x41, 0x53, 0x44]),
    Some(&ARROWS),
    Some(&PAD),
    Some(&[0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c, 0x5d]),
    Some(&[0x2d, 0x24, 0x21, 0x2e, 0x23, 0x22]),
    None,
];

pub fn on_pad(x: f32) -> bool {
    x > 18.25
}

pub fn key_name(vk: u32) -> String {
    let named = match vk {
        0x1b => "Esc",
        0x2c => "PrtSc",
        0x91 => "ScrLk",
        0x13 => "Pause",
        0xc0 => "`",
        0xbd | 0x6d => "-",
        0xbb => "=",
        0x08 => "Backspace",
        0x2d => "Ins",
        0x24 => "Home",
        0x21 => "PgUp",
        0x90 => "Num",
        0x6f | 0xbf => "/",
        0x6a => "*",
        0x09 => "Tab",
        0xdb => "[",
        0xdd => "]",
        0xdc => "\\",
        0x2e => "Del",
        0x23 => "End",
        0x22 => "PgDn",
        0x6b => "+",
        0x14 => "Caps",
        0xba => ";",
        0xde => "'",
        0x0d => "Enter",
        0xa0 | 0xa1 => "Shift",
        0xbc => ",",
        0xbe | 0x6e => ".",
        0x26 => "\u{2191}",
        0xa2 | 0xa3 => "Ctrl",
        0x5b | 0x5c => "Win",
        0xa4 | 0xa5 => "Alt",
        0x20 => "Space",
        0x5d => "Menu",
        0x25 => "\u{2190}",
        0x28 => "\u{2193}",
        0x27 => "\u{2192}",
        _ => "",
    };
    if !named.is_empty() {
        return named.into();
    }
    match vk {
        0x70..=0x7b => format!("F{}", vk - 0x6f),
        0x60..=0x69 => (vk - 0x60).to_string(),
        _ => char::from_u32(vk).map(String::from).unwrap_or_default(),
    }
}

pub fn key_short(vk: u32) -> String {
    let short = match vk {
        0x2c => "Prt",
        0x91 => "Scr",
        0x13 => "Brk",
        0x08 => "Bksp",
        0x2d => "Ins",
        0x24 => "Hm",
        0x21 => "PgU",
        0x22 => "PgD",
        0x90 => "Num",
        0x2e => "Del",
        0xa0 | 0xa1 => "Shift",
        0x5b | 0x5c => "Win",
        0x5d => "Mnu",
        0xa2 | 0xa3 => "Ctl",
        _ => return key_name(vk),
    };
    short.into()
}

/// The key a Slint key event's text stands for. Number pad keys arrive as the main row's.
pub fn vk_from_text(text: &str) -> Option<u32> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let vk = match c {
        'a'..='z' => c.to_ascii_uppercase() as u32,
        'A'..='Z' | '0'..='9' => c as u32,
        ')' => 0x30,
        '!' => 0x31,
        '@' => 0x32,
        '#' => 0x33,
        '$' => 0x34,
        '%' => 0x35,
        '^' => 0x36,
        '&' => 0x37,
        '*' => 0x38,
        '(' => 0x39,
        ' ' => 0x20,
        '\u{8}' => 0x08,
        '\n' | '\r' => 0x0d,
        '\u{1b}' => 0x1b,
        '\u{7f}' => 0x2e,
        '\u{10}' => 0xa0,
        '\u{15}' => 0xa1,
        '\u{11}' => 0xa2,
        '\u{16}' => 0xa3,
        '\u{12}' => 0xa4,
        '\u{13}' => 0xa5,
        '\u{14}' => 0x14,
        '\u{17}' => 0x5b,
        '\u{18}' => 0x5c,
        '\u{F700}' => 0x26,
        '\u{F701}' => 0x28,
        '\u{F702}' => 0x25,
        '\u{F703}' => 0x27,
        '\u{F704}'..='\u{F70F}' => 0x70 + (c as u32 - 0xF704),
        '\u{F727}' => 0x2d,
        '\u{F729}' => 0x24,
        '\u{F72B}' => 0x23,
        '\u{F72C}' => 0x21,
        '\u{F72D}' => 0x22,
        '\u{F72F}' => 0x91,
        '\u{F730}' => 0x13,
        '\u{F731}' => 0x2c,
        '\u{F735}' => 0x5d,
        '`' | '~' => 0xc0,
        '-' | '_' => 0xbd,
        '=' | '+' => 0xbb,
        '[' | '{' => 0xdb,
        ']' | '}' => 0xdd,
        '\\' | '|' => 0xdc,
        ';' | ':' => 0xba,
        '\'' | '"' => 0xde,
        ',' | '<' => 0xbc,
        '.' | '>' => 0xbe,
        '/' | '?' => 0xbf,
        _ => return None,
    };
    Some(vk)
}
