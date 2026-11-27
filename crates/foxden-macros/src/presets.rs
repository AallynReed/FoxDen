//! Ready-made macros most people want, added with one click and then given a key or button.

use crate::keys::by_name;
use crate::model::{Macro, MediaKey, Mode, Step};

pub struct Preset {
    pub id: &'static str,
    pub group: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub make: fn() -> Macro,
}

pub const GROUPS: [&str; 4] = ["Editing", "Windows", "Media", "More"];

fn key(name: &str) -> u16 {
    by_name(name).unwrap_or_else(|| panic!("no key {name}"))
}

fn chord(name: &str, keys: &[&str]) -> Macro {
    let (last, mods) = keys.split_last().expect("a chord has a key");
    let mods: Vec<u16> = mods.iter().map(|k| key(k)).collect();
    Macro::chord(name, &mods, key(last))
}

fn media(name: &str, k: MediaKey) -> Macro {
    Macro { name: name.into(), press: vec![Step::Media { key: k }], ..Default::default() }
}

pub const ALL: &[Preset] = &[
    Preset { id: "copy", group: "Editing", name: "Copy", about: "Ctrl+C", make: || chord("Copy", &["Ctrl", "C"]) },
    Preset { id: "paste", group: "Editing", name: "Paste", about: "Ctrl+V", make: || chord("Paste", &["Ctrl", "V"]) },
    Preset { id: "cut", group: "Editing", name: "Cut", about: "Ctrl+X", make: || chord("Cut", &["Ctrl", "X"]) },
    Preset { id: "undo", group: "Editing", name: "Undo", about: "Ctrl+Z", make: || chord("Undo", &["Ctrl", "Z"]) },
    Preset { id: "redo", group: "Editing", name: "Redo", about: "Ctrl+Y", make: || chord("Redo", &["Ctrl", "Y"]) },
    Preset { id: "select-all", group: "Editing", name: "Select all", about: "Ctrl+A", make: || chord("Select all", &["Ctrl", "A"]) },
    Preset { id: "save", group: "Editing", name: "Save", about: "Ctrl+S", make: || chord("Save", &["Ctrl", "S"]) },
    Preset { id: "clipboard", group: "Editing", name: "Clipboard history", about: "Win+V", make: || chord("Clipboard history", &["Win", "V"]) },
    Preset { id: "emoji", group: "Editing", name: "Emoji panel", about: "Win+.", make: || chord("Emoji panel", &["Win", "."]) },
    Preset { id: "snip", group: "Windows", name: "Screenshot an area", about: "Win+Shift+S", make: || chord("Screenshot an area", &["Win", "Shift", "S"]) },
    Preset { id: "desktop", group: "Windows", name: "Show desktop", about: "Win+D", make: || chord("Show desktop", &["Win", "D"]) },
    Preset { id: "switch", group: "Windows", name: "Switch window", about: "Alt+Tab", make: || chord("Switch window", &["Alt", "Tab"]) },
    Preset { id: "task-view", group: "Windows", name: "Task view", about: "Win+Tab", make: || chord("Task view", &["Win", "Tab"]) },
    Preset { id: "task-manager", group: "Windows", name: "Task Manager", about: "Ctrl+Shift+Esc", make: || chord("Task Manager", &["Ctrl", "Shift", "Esc"]) },
    Preset { id: "explorer", group: "Windows", name: "File Explorer", about: "Win+E", make: || chord("File Explorer", &["Win", "E"]) },
    Preset { id: "desk-left", group: "Windows", name: "Desktop to the left", about: "Ctrl+Win+Left", make: || chord("Desktop to the left", &["Ctrl", "Win", "Left"]) },
    Preset { id: "desk-right", group: "Windows", name: "Desktop to the right", about: "Ctrl+Win+Right", make: || chord("Desktop to the right", &["Ctrl", "Win", "Right"]) },
    Preset { id: "close-tab", group: "Windows", name: "Close tab", about: "Ctrl+W", make: || chord("Close tab", &["Ctrl", "W"]) },
    Preset { id: "reopen-tab", group: "Windows", name: "Reopen closed tab", about: "Ctrl+Shift+T", make: || chord("Reopen closed tab", &["Ctrl", "Shift", "T"]) },
    Preset { id: "play", group: "Media", name: "Play or pause", about: "Media key", make: || media("Play or pause", MediaKey::PlayPause) },
    Preset { id: "next", group: "Media", name: "Next track", about: "Media key", make: || media("Next track", MediaKey::Next) },
    Preset { id: "previous", group: "Media", name: "Previous track", about: "Media key", make: || media("Previous track", MediaKey::Previous) },
    Preset { id: "mute", group: "Media", name: "Mute", about: "Media key", make: || media("Mute", MediaKey::Mute) },
    Preset { id: "vol-up", group: "Media", name: "Volume up", about: "Media key", make: || media("Volume up", MediaKey::VolumeUp) },
    Preset { id: "vol-down", group: "Media", name: "Volume down", about: "Media key", make: || media("Volume down", MediaKey::VolumeDown) },
    Preset {
        id: "rapid-click",
        group: "More",
        name: "Rapid click",
        about: "Clicks 20 times a second while held",
        make: || Macro {
            name: "Rapid click".into(),
            mode: Mode::RepeatWhileHeld,
            press: vec![
                Step::Button { button: 1, down: true },
                Step::Delay { ms: 15 },
                Step::Button { button: 1, down: false },
                Step::Delay { ms: 35 },
            ],
            ..Default::default()
        },
    },
    Preset {
        id: "double-click",
        group: "More",
        name: "Double click",
        about: "Two left clicks",
        make: || Macro {
            name: "Double click".into(),
            press: vec![
                Step::Button { button: 1, down: true },
                Step::Button { button: 1, down: false },
                Step::Delay { ms: 40 },
                Step::Button { button: 1, down: true },
                Step::Button { button: 1, down: false },
            ],
            ..Default::default()
        },
    },
    Preset {
        id: "snippet",
        group: "More",
        name: "Type a snippet",
        about: "Types text you choose, such as your email",
        make: || Macro { name: "Type a snippet".into(), press: vec![Step::Text { text: "Your text here".into() }], ..Default::default() },
    },
    Preset {
        id: "chat-line",
        group: "More",
        name: "Type, then Enter",
        about: "Types a line and sends it, for chat",
        make: || {
            let mut m = Macro { name: "Type, then Enter".into(), press: vec![Step::Text { text: "gg".into() }], ..Default::default() };
            m.press.extend(chord("", &["Enter"]).press);
            m
        },
    },
];

pub fn get(id: &str) -> Option<&'static Preset> {
    ALL.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_valid_and_grouped() {
        for p in ALL {
            let m = (p.make)();
            assert_eq!(m.name, p.name);
            m.check().unwrap();
            assert!(m.playable(), "{}", p.id);
            assert!(GROUPS.contains(&p.group), "{}", p.id);
            assert_eq!(ALL.iter().filter(|q| q.id == p.id).count(), 1);
        }
        assert_eq!(get("copy").map(|p| p.about), Some("Ctrl+C"));
    }
}
