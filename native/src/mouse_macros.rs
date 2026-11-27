//! Macros stored in a mouse's own memory and put on its buttons.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use foxden_hid::onboard::{self, Binding, Op};
use foxden_macros::{Macro, Mode, Step};
use serde::{Deserialize, Serialize};

use crate::log;
use crate::state::lock;

/// Text the mouse can type, as US-layout keys.
fn text_key(c: char) -> Option<(bool, u8)> {
    const PLAIN: &str = "1234567890";
    const SHIFTED: &str = "!@#$%^&*()";
    const PUNCT: [(char, char, u8); 11] = [
        ('-', '_', 0x2D),
        ('=', '+', 0x2E),
        ('[', '{', 0x2F),
        (']', '}', 0x30),
        ('\\', '|', 0x31),
        (';', ':', 0x33),
        ('\'', '"', 0x34),
        ('`', '~', 0x35),
        (',', '<', 0x36),
        ('.', '>', 0x37),
        ('/', '?', 0x38),
    ];
    match c {
        'a'..='z' => Some((false, 0x04 + (c as u8 - b'a'))),
        'A'..='Z' => Some((true, 0x04 + (c as u8 - b'A'))),
        ' ' => Some((false, 0x2C)),
        '\n' => Some((false, 0x28)),
        '\t' => Some((false, 0x2B)),
        _ => {
            if let Some(i) = PLAIN.find(c) {
                return Some((false, 0x1E + i as u8));
            }
            if let Some(i) = SHIFTED.find(c) {
                return Some((true, 0x1E + i as u8));
            }
            PUNCT.iter().find_map(|&(p, s, k)| (c == p).then_some((false, k)).or((c == s).then_some((true, k))))
        }
    }
}

const SHIFT_BIT: u8 = 0x02;

fn wait(ms: u32, out: &mut Vec<Op>) {
    let mut left = ms;
    while left > 0 {
        let n = left.min(onboard::DELAY_MAX_MS);
        out.push(Op::Delay(n as u16));
        left -= n;
    }
}

/// What the mouse can play: keys, text and waits, once per press.
pub fn ops(m: &Macro) -> Result<Vec<Op>, String> {
    if m.mode != Mode::Once {
        return Err("The mouse can only play a macro once per press. Set the mode to Once.".into());
    }
    let mut out = Vec::new();
    let mut held = 0u8;
    for (i, s) in m.press.iter().enumerate() {
        if i > 0
            && let Some(d) = m.default_delay_ms
            && !matches!(s, Step::Delay { .. })
            && !matches!(m.press[i - 1], Step::Delay { .. })
        {
            wait(d, &mut out);
        }
        match s {
            Step::Key { usage, down } if foxden_macros::keys::is_modifier(*usage) => {
                let bit = 1u8 << (usage - 0xE0);
                if *down {
                    held |= bit;
                    out.push(Op::Press { mods: held, key: 0 });
                } else {
                    held &= !bit;
                    out.push(Op::Release { mods: bit, key: 0 });
                }
            }
            Step::Key { usage, down } => {
                let key = u8::try_from(*usage).map_err(|_| "That key can't be stored on the mouse.")?;
                out.push(if *down { Op::Press { mods: held, key } } else { Op::Release { mods: held, key } });
            }
            Step::Delay { ms } => wait(*ms, &mut out),
            Step::Text { text } => {
                for c in text.chars() {
                    let (shift, key) = text_key(c).ok_or_else(|| {
                        format!("The mouse can only type plain English letters, digits and symbols, not \"{c}\".")
                    })?;
                    let mods = held | if shift { SHIFT_BIT } else { 0 };
                    out.push(Op::Press { mods, key });
                    out.push(Op::Release { mods, key });
                }
            }
            Step::Button { .. } | Step::Wheel { .. } | Step::Media { .. } => {
                return Err("The mouse can store keys, text and waits, not clicks, wheel turns or media keys.".into());
            }
        }
    }
    if out.is_empty() {
        return Err("This macro has no steps.".into());
    }
    Ok(out)
}

pub const NOTE: &str = "The mouse plays its stored macros itself, once per press.";

/// Which saved macro each mouse button holds, by mouse and slot.
#[derive(Default, Serialize, Deserialize)]
struct Names(BTreeMap<String, BTreeMap<u8, String>>);

#[derive(Clone, Debug, Serialize)]
pub struct ButtonView {
    pub slot: u8,
    pub label: String,
    pub factory: String,
    pub macro_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MouseView {
    pub id: String,
    pub name: String,
    pub onboard: bool,
    pub buttons: Vec<ButtonView>,
}

pub struct MouseMacros {
    path: PathBuf,
    names: Mutex<Names>,
    io: Mutex<()>,
}

/// Plain names for the buttons, from what they do from the factory.
fn button_name(slot: u8, factory: Binding) -> String {
    format!("Button {} ({})", slot + 1, factory.label())
}

impl MouseMacros {
    pub fn open(path: PathBuf) -> Self {
        let names = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        MouseMacros { path, names: Mutex::new(names), io: Mutex::new(()) }
    }

    fn save(&self) {
        let json = serde_json::to_vec_pretty(&*lock(&self.names)).unwrap_or_default();
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            log::warn("macros", "Couldn't save the names of the mouse macros");
        }
    }

    fn view(&self, m: &onboard::Mouse) -> Result<MouseView, String> {
        let names = lock(&self.names);
        let mine = names.0.get(&m.id);
        let buttons = onboard::list(m)?
            .into_iter()
            .map(|b| {
                let macro_name = match b.now {
                    Binding::Macro { .. } => {
                        Some(mine.and_then(|n| n.get(&b.slot)).cloned().unwrap_or_else(|| "Stored macro".into()))
                    }
                    _ => None,
                };
                ButtonView {
                    slot: b.slot,
                    label: button_name(b.slot, b.factory),
                    factory: b.factory.label(),
                    macro_name,
                }
            })
            .collect();
        Ok(MouseView { id: m.id.clone(), name: m.name.clone(), onboard: m.mode().ok() == Some(1), buttons })
    }

    /// Every mouse that can hold macros, read now. Slow: run it off the UI thread.
    pub fn scan(&self) -> Vec<MouseView> {
        let _io = lock(&self.io);
        onboard::mice()
            .iter()
            .filter_map(|m| self.view(m).map_err(|e| log::warn("macros", format!("{}: {e}", m.name))).ok())
            .collect()
    }

    fn with_mouse<T>(&self, id: &str, f: impl FnOnce(&onboard::Mouse) -> Result<T, String>) -> Result<T, String> {
        let _io = lock(&self.io);
        let mice = onboard::mice();
        let m = mice
            .iter()
            .find(|m| m.id == id || id.is_empty())
            .ok_or("That mouse isn't connected, or it's asleep. Move it and try again.")?;
        f(m)
    }

    pub fn store(&self, mouse: &str, slot: u8, m: &Macro) -> Result<String, String> {
        let ops = ops(m)?;
        let (id, name) = self.with_mouse(mouse, |mm| {
            let sector = onboard::store(mm, slot, &ops)?;
            log::info("macros", format!("Stored \"{}\" on {} button {} (sector {sector})", m.name, mm.name, slot + 1));
            Ok((mm.id.clone(), mm.name.clone()))
        })?;
        lock(&self.names).0.entry(id).or_default().insert(slot, m.name.clone());
        self.save();
        Ok(format!("\"{}\" is on {name} button {}.", m.name, slot + 1))
    }

    pub fn reset(&self, mouse: &str, slot: u8) -> Result<String, String> {
        let (id, name) = self.with_mouse(mouse, |mm| {
            onboard::reset(mm, slot)?;
            log::info("macros", format!("Put {} button {} back to its factory setting", mm.name, slot + 1));
            Ok((mm.id.clone(), mm.name.clone()))
        })?;
        if let Some(n) = lock(&self.names).0.get_mut(&id) {
            n.remove(&slot);
        }
        self.save();
        Ok(format!("{name} button {} does what it did from the factory again.", slot + 1))
    }
}

pub fn start(app: &tauri::AppHandle) {
    use tauri::Manager;
    app.manage(MouseMacros::open(crate::config::dir().join("mouse-macros.json")));
}

pub fn get(app: &tauri::AppHandle) -> Result<tauri::State<'static, MouseMacros>, String> {
    use tauri::Manager;
    app.try_state::<MouseMacros>().ok_or_else(|| "FoxDen is still starting.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_text_and_waits_become_mouse_steps() {
        let mut m = Macro::chord("Copy", &[0xE0], 0x06);
        m.press.push(Step::Text { text: "Hi!".into() });
        m.press.push(Step::Delay { ms: 70_000 });
        let o = ops(&m).unwrap();
        assert_eq!(o[..4], [
            Op::Press { mods: 0x01, key: 0 },
            Op::Press { mods: 0x01, key: 0x06 },
            Op::Release { mods: 0x01, key: 0x06 },
            Op::Release { mods: 0x01, key: 0 },
        ]);
        let two = ops(&Macro::chord("Find", &[0xE0, 0xE1], 0x09)).unwrap();
        assert_eq!(two[2], Op::Press { mods: 0x03, key: 0x09 });
        assert_eq!(o[4], Op::Press { mods: 0x02, key: 0x0B });
        assert_eq!(o[6], Op::Press { mods: 0, key: 0x0C });
        assert_eq!(o[8], Op::Press { mods: 0x02, key: 0x1E });
        assert_eq!(o[10..], [Op::Delay(65535), Op::Delay(4465)]);
    }

    #[test]
    fn what_the_mouse_cant_do_is_refused() {
        let click = Macro { name: "c".into(), press: vec![Step::Button { button: 1, down: true }], ..Default::default() };
        assert!(ops(&click).unwrap_err().contains("not clicks"));
        let repeat = Macro { mode: Mode::Toggle, ..Macro::chord("x", &[], 4) };
        assert!(ops(&repeat).unwrap_err().contains("once per press"));
        let accent = Macro { name: "t".into(), press: vec![Step::Text { text: "é".into() }], ..Default::default() };
        assert!(ops(&accent).unwrap_err().contains("\"é\""));
        assert!(ops(&Macro { name: "e".into(), ..Default::default() }).is_err());
    }

    #[test]
    fn default_waits_go_between_steps() {
        let mut m = Macro::chord("a", &[], 4);
        m.default_delay_ms = Some(10);
        assert_eq!(ops(&m).unwrap(), [Op::Press { mods: 0, key: 4 }, Op::Delay(10), Op::Release { mods: 0, key: 4 }]);
    }
}
