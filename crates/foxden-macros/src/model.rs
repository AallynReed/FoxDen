use serde::{Deserialize, Serialize};

use crate::keys;

pub const NAME_MAX: usize = 40;
pub const STEPS_MAX: usize = 1000;
pub const DELAY_MAX: u32 = 60_000;
pub const TEXT_MAX: usize = 1000;
pub const WHEEL_MAX: i32 = 100;

/// How a macro reacts to its hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Plays `press` once per press.
    #[default]
    Once,
    /// Plays `press` on press, loops `hold` while held, plays `release` on release.
    PressHoldRelease,
    /// Loops `press` while the hotkey is held.
    RepeatWhileHeld,
    /// One press starts looping `press`, the next stops it.
    Toggle,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Once, Mode::PressHoldRelease, Mode::RepeatWhileHeld, Mode::Toggle];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Once => "Once",
            Mode::PressHoldRelease => "Press, hold, release",
            Mode::RepeatWhileHeld => "Repeat while held",
            Mode::Toggle => "Toggle",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Mode::Once => "once",
            Mode::PressHoldRelease => "press_hold_release",
            Mode::RepeatWhileHeld => "repeat_while_held",
            Mode::Toggle => "toggle",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        Mode::ALL.into_iter().find(|m| m.code() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKey {
    PlayPause,
    Stop,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    Mute,
}

impl MediaKey {
    pub const ALL: [MediaKey; 7] = [
        MediaKey::PlayPause,
        MediaKey::Stop,
        MediaKey::Next,
        MediaKey::Previous,
        MediaKey::VolumeUp,
        MediaKey::VolumeDown,
        MediaKey::Mute,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MediaKey::PlayPause => "Play/pause",
            MediaKey::Stop => "Stop",
            MediaKey::Next => "Next track",
            MediaKey::Previous => "Previous track",
            MediaKey::VolumeUp => "Volume up",
            MediaKey::VolumeDown => "Volume down",
            MediaKey::Mute => "Mute",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            MediaKey::PlayPause => "play_pause",
            MediaKey::Stop => "stop",
            MediaKey::Next => "next",
            MediaKey::Previous => "previous",
            MediaKey::VolumeUp => "volume_up",
            MediaKey::VolumeDown => "volume_down",
            MediaKey::Mute => "mute",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        MediaKey::ALL.into_iter().find(|m| m.code() == s)
    }
}

/// One thing a macro does. Keys are USB HID usages (keyboard page 0x07).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Key { usage: u16, down: bool },
    /// 1 left, 2 right, 3 middle, 4 back, 5 forward.
    Button { button: u8, down: bool },
    /// Positive is up, or right when horizontal.
    Wheel { horizontal: bool, ticks: i32 },
    Delay { ms: u32 },
    Text { text: String },
    Media { key: MediaKey },
}

impl Step {
    pub fn describe(&self) -> String {
        let updown = |d: bool| if d { "down" } else { "up" };
        match self {
            Step::Key { usage, down } => format!("{} {}", keys::name(*usage), updown(*down)),
            Step::Button { button, down } => format!("{} {}", button_name(*button), updown(*down)),
            Step::Wheel { horizontal, ticks } => {
                let way = match (horizontal, *ticks > 0) {
                    (false, true) => "up",
                    (false, false) => "down",
                    (true, true) => "right",
                    (true, false) => "left",
                };
                format!("Wheel {way} x{}", ticks.abs())
            }
            Step::Delay { ms } => format!("Wait {ms} ms"),
            Step::Text { text } => format!("Type \"{text}\""),
            Step::Media { key } => key.label().to_string(),
        }
    }

    fn check(&self) -> Result<(), String> {
        match self {
            Step::Key { usage, .. } if keys::scancode(*usage).is_none() && *usage != keys::PAUSE_USAGE => {
                Err(format!("Key usage {usage:#x} isn't a key FoxDen can send."))
            }
            Step::Button { button, .. } if !(1..=5).contains(button) => Err("Mouse buttons go from 1 to 5.".into()),
            Step::Wheel { ticks, .. } if *ticks == 0 || ticks.abs() > WHEEL_MAX => {
                Err(format!("Wheel ticks go from 1 to {WHEEL_MAX}, up or down."))
            }
            Step::Delay { ms } if *ms > DELAY_MAX => Err(format!("A wait can be at most {DELAY_MAX} ms.")),
            Step::Text { text } if text.is_empty() || text.chars().count() > TEXT_MAX => {
                Err(format!("Text to type must be 1 to {TEXT_MAX} characters."))
            }
            _ => Ok(()),
        }
    }
}

pub fn button_name(b: u8) -> &'static str {
    match b {
        1 => "Left button",
        2 => "Right button",
        3 => "Middle button",
        4 => "Back button",
        5 => "Forward button",
        _ => "Mouse button",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Macro {
    pub name: String,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub press: Vec<Step>,
    #[serde(default)]
    pub hold: Vec<Step>,
    #[serde(default)]
    pub release: Vec<Step>,
    /// Pause put between steps that don't wait themselves.
    #[serde(default)]
    pub default_delay_ms: Option<u32>,
}

impl Macro {
    pub fn steps(&self) -> usize {
        self.press.len() + self.hold.len() + self.release.len()
    }

    pub fn playable(&self) -> bool {
        match self.mode {
            Mode::PressHoldRelease => self.steps() > 0,
            _ => !self.press.is_empty(),
        }
    }

    /// A key chord: modifiers down, key down, key up, modifiers up.
    pub fn chord(name: &str, modifiers: &[u16], usage: u16) -> Self {
        let mut press: Vec<Step> = modifiers.iter().map(|&u| Step::Key { usage: u, down: true }).collect();
        press.push(Step::Key { usage, down: true });
        press.push(Step::Key { usage, down: false });
        press.extend(modifiers.iter().rev().map(|&u| Step::Key { usage: u, down: false }));
        Macro { name: name.into(), press, ..Default::default() }
    }

    pub fn check(&self) -> Result<(), String> {
        let n = self.name.trim().chars().count();
        if n == 0 || n > NAME_MAX {
            return Err(format!("A macro name must be 1 to {NAME_MAX} characters."));
        }
        if self.steps() > STEPS_MAX {
            return Err(format!("A macro can have at most {STEPS_MAX} steps."));
        }
        if self.default_delay_ms.is_some_and(|d| d > DELAY_MAX) {
            return Err(format!("The default delay can be at most {DELAY_MAX} ms."));
        }
        self.press.iter().chain(&self.hold).chain(&self.release).try_for_each(Step::check)
    }
}

/// A key or mouse button Windows reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    Key(u16),
    Button(u8),
}

impl Input {
    pub fn label(self) -> String {
        match self {
            Input::Key(u) => keys::name(u).to_string(),
            Input::Button(b) => button_name(b).to_string(),
        }
    }
}

pub const CTRL: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const WIN: u8 = 8;
const MOD_NAMES: [(u8, &str); 4] = [(CTRL, "Ctrl"), (SHIFT, "Shift"), (ALT, "Alt"), (WIN, "Win")];

/// Left and right modifiers held, one bit per usage 0xE0..=0xE7, folded to CTRL/SHIFT/ALT/WIN.
pub fn fold(held: u8) -> u8 {
    (held | (held >> 4)) & 0x0F
}

/// What starts a macro: an input, with the modifiers that must be held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hotkey {
    pub input: Input,
    #[serde(default)]
    pub mods: u8,
}

impl Hotkey {
    pub fn label(self) -> String {
        let mut parts: Vec<String> =
            MOD_NAMES.iter().filter(|(b, _)| self.mods & b != 0).map(|(_, n)| n.to_string()).collect();
        parts.push(self.input.label());
        parts.join("+")
    }

    /// Reads "Ctrl+Shift+F13", "F14" or "Back button".
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
        let (last, mods) = parts.split_last().ok_or("Name a key or mouse button.")?;
        let mut bits = 0;
        for m in mods {
            let bit = MOD_NAMES
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(m) || (*n == "Ctrl" && m.eq_ignore_ascii_case("control")))
                .map(|(b, _)| *b)
                .ok_or_else(|| format!("{m} isn't Ctrl, Shift, Alt or Win."))?;
            bits |= bit;
        }
        let input = (1..=5u8)
            .find(|b| button_name(*b).eq_ignore_ascii_case(last) || format!("mouse{b}").eq_ignore_ascii_case(last))
            .map(Input::Button)
            .or_else(|| keys::by_name(last).map(Input::Key))
            .ok_or_else(|| format!("{last} isn't a key or mouse button FoxDen knows."))?;
        let h = Hotkey { input, mods: bits };
        h.check()?;
        Ok(h)
    }

    pub fn check(self) -> Result<(), String> {
        match self.input {
            Input::Key(u) if u == keys::PAUSE_USAGE => Err("Pause stops all macros, so it can't start one.".into()),
            Input::Key(u) if keys::is_modifier(u) => Err("A modifier on its own can't start a macro.".into()),
            Input::Key(u) if keys::scancode(u).is_none() => Err("FoxDen doesn't know that key.".into()),
            Input::Button(1 | 2) if self.mods == 0 => {
                Err("Left and right click need a modifier, such as Ctrl, to start a macro.".into())
            }
            Input::Button(b) if !(1..=5).contains(&b) => Err("Mouse buttons go from 1 to 5.".into()),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(rename = "macro")]
    pub macro_: Macro,
    #[serde(default)]
    pub hotkey: Option<Hotkey>,
    /// Hide the hotkey from the app in front.
    #[serde(default)]
    pub block: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    /// Hotkeys are watched only while this is on.
    #[serde(default)]
    pub enabled: bool,
    /// The note on anti-cheat and elevated apps was read.
    #[serde(default)]
    pub noted: bool,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Library {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    pub fn new_id(&self) -> String {
        (1..).map(|i| format!("m{i}")).find(|id| !self.entries.iter().any(|e| &e.id == id)).unwrap()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The entry a hotkey starts.
    pub fn bound(&self, hotkey: Hotkey) -> Option<&Entry> {
        self.entries.iter().find(|e| e.hotkey == Some(hotkey) && e.macro_.playable())
    }

    /// Adds or replaces an entry after checking it; returns its id.
    pub fn put(&mut self, mut entry: Entry) -> Result<String, String> {
        entry.macro_.name = entry.macro_.name.trim().to_string();
        entry.macro_.check()?;
        if let Some(h) = entry.hotkey {
            h.check()?;
            if let Some(other) = self.entries.iter().find(|e| e.id != entry.id && e.hotkey == Some(h)) {
                return Err(format!("{} already starts \"{}\".", h.label(), other.macro_.name));
            }
        }
        if self.entries.iter().any(|e| e.id != entry.id && e.macro_.name.eq_ignore_ascii_case(&entry.macro_.name)) {
            return Err(format!("A macro named \"{}\" already exists.", entry.macro_.name));
        }
        if entry.id.is_empty() {
            entry.id = self.new_id();
        }
        let id = entry.id.clone();
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) => *e = entry,
            None => self.entries.push(entry),
        }
        Ok(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Entry> {
        let i = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, hotkey: Option<Hotkey>) -> Entry {
        Entry { id: String::new(), macro_: Macro::chord(name, &[0xE0], 0x06), hotkey, block: false }
    }

    #[test]
    fn hotkeys_parse_and_print() {
        let h = Hotkey::parse("ctrl + Shift+F13").unwrap();
        assert_eq!(h, Hotkey { input: Input::Key(0x68), mods: CTRL | SHIFT });
        assert_eq!(h.label(), "Ctrl+Shift+F13");
        assert_eq!(Hotkey::parse("Forward button").unwrap().input, Input::Button(5));
        assert_eq!(Hotkey::parse("Alt+mouse4").unwrap(), Hotkey { input: Input::Button(4), mods: ALT });
        assert!(Hotkey::parse("Pause").is_err());
        assert!(Hotkey::parse("Left button").is_err());
        assert!(Hotkey::parse("Ctrl+Left button").is_ok());
        assert!(Hotkey::parse("Ctrl").is_err());
        assert!(Hotkey::parse("Hyper+A").is_err());
        assert!(Hotkey::parse("").is_err());
    }

    #[test]
    fn modifiers_fold_both_sides() {
        assert_eq!(fold(0b0001_0000), CTRL);
        assert_eq!(fold(0b0000_0110), SHIFT | ALT);
        assert_eq!(fold(0b1000_1000), WIN);
    }

    #[test]
    fn put_checks_names_and_hotkeys() {
        let mut lib = Library::default();
        let f13 = Hotkey { input: Input::Key(0x68), mods: 0 };
        let a = lib.put(entry("Copy", Some(f13))).unwrap();
        assert_eq!(a, "m1");
        assert!(lib.put(entry("Other", Some(f13))).unwrap_err().contains("already starts"));
        assert!(lib.put(entry(" copy ", None)).unwrap_err().contains("already exists"));
        let b = lib.put(entry("Paste", None)).unwrap();
        assert_eq!(b, "m2");
        let mut edit = lib.get(&a).unwrap().clone();
        edit.macro_.name = "Copy all".into();
        assert_eq!(lib.put(edit).unwrap(), a);
        assert_eq!(lib.entries.len(), 2);
        assert_eq!(lib.bound(f13).unwrap().macro_.name, "Copy all");
        assert!(lib.remove(&a).is_some());
        assert!(lib.bound(f13).is_none());
        assert_eq!(lib.new_id(), "m1");
    }

    #[test]
    fn macros_are_checked() {
        let mut m = Macro::chord("x", &[], 0x04);
        assert!(m.check().is_ok() && m.playable());
        m.press.push(Step::Delay { ms: DELAY_MAX + 1 });
        assert!(m.check().is_err());
        m.press.pop();
        m.press.push(Step::Text { text: String::new() });
        assert!(m.check().is_err());
        m.press.pop();
        m.press.push(Step::Wheel { horizontal: false, ticks: 0 });
        assert!(m.check().is_err());
        m.press.pop();
        m.press.push(Step::Button { button: 9, down: true });
        assert!(m.check().is_err());
        m.press.pop();
        m.name = " ".into();
        assert!(m.check().is_err());
        let hold_only = Macro { name: "h".into(), mode: Mode::PressHoldRelease, hold: m.press.clone(), ..Default::default() };
        assert!(hold_only.playable());
        assert!(!Macro { mode: Mode::Toggle, ..hold_only }.playable());
    }

    #[test]
    fn library_round_trips() {
        let mut lib = Library { enabled: true, ..Default::default() };
        lib.put(entry("Copy", Some(Hotkey { input: Input::Button(4), mods: CTRL }))).unwrap();
        lib.entries[0].macro_.press.push(Step::Media { key: MediaKey::Mute });
        lib.entries[0].macro_.press.push(Step::Text { text: "hé😀".into() });
        let dir = std::env::temp_dir().join(format!("foxden-macros-{}", std::process::id()));
        let path = dir.join("macros.json");
        lib.save(&path).unwrap();
        assert_eq!(Library::load(&path), lib);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"type\": \"media\"") && text.contains("\"macro\""));
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(Library::load(std::path::Path::new("no/such/file.json")), Library::default());
    }
}
